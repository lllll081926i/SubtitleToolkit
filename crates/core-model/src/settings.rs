//! 36 §1 的 **AppSettings**：全局偏好，跨会话保留。
//!
//! ## 为什么现在做，以及为什么放在 core-model
//!
//! 28 §4 把「settings/presets/logs 的落盘」列为便携包仍未实现的一项，而
//! 36 §1 的配置分层里 AppSettings 是**最底层**：ToolDefaults 与 Preset 都会从它取默认。
//! 放到 CLI 或 GUI 里各写一份，两份默认值迟早不一致，而用户看到的正是"同一个设置
//! 在两个入口表现不同"这种报告。放 core-model 让两个前端共用一个定义。
//!
//! ## 五条纪律
//!
//! **一、文件坏了不能让人开不了程序。**
//! 设置是**偏好**，不是数据。解析失败就退回默认值并把失败说出来（见
//! [`SettingsLoad::problems`]），绝不报错退出。用户丢的是偏好，不是字幕。
//!
//! **二、未知字段必须保留。**
//! 36 §3 明确要求「未知字段保留，便于新版/旧版往返」。旧版程序读一份新版写的设置、
//! 原样写回时若丢掉不认识的字段，用户在新版里的选择就静默消失了。
//! 所以这里持有一份 `extra` 原始 JSON 并在写回时合并。
//! 注意这与 batch spec 的 `deny_unknown_fields` 是**相反**的策略，也有相反的理由：
//! spec 拼错字段会导致跑错任务（必须拒绝），设置里有陌生字段只说明它来自别的版本
//! （必须保留）。
//!
//! **三、只向前迁移，迁移前备份。**
//! `schema` 比当前版本低就走迁移链；比当前高就**只读不写**并说明原因——
//! 新版写的设置里可能有旧版无法表达的语义，强行写回等于降级破坏。
//!
//! **四、写入必须是原子的。**
//! 同目录临时文件 + 改名。设置文件被截断一半会让下一次启动退回默认值，
//! 而用户不知道自己刚设的东西为什么没了。
//!
//! **五、坏文件先留证据再重建。**
//! 解析失败时把原文件改名成 `<名>.bad-<时间戳>` 而不是直接覆盖：用户可能想看一眼
//! 自己丢的是什么。这也是"不硬删用户数据"那条纪律在设置文件上的落法。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::datadir::DataDir;
use crate::error::{Domain, Result, StkError};

/// 当前设置文件的 schema 版本。加字段不改它；**改语义**才加。
pub const CURRENT_SCHEMA: u32 = 1;

/// 设置文件名（在数据目录里）。
pub const FILE_NAME: &str = "settings.json";

/// 36 §1 的 AppSettings：全局偏好。
///
/// 字段刻意选少：只放**两个前端都真的会读**的项。把"将来可能有用的东西"先塞进来，
/// 会变成一份没人读、也没人敢删的清单。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSettings {
    /// 数据文件格式版本。读到时若高于 [`CURRENT_SCHEMA`]，只读不写。
    pub schema: u32,
    /// 界面语言；`None` 表示跟随系统。报告与错误文案都用这个词。
    pub ui_language: Option<String>,
    /// 日志级别（22 §）。字符串而不是枚举：日志过滤是 tracing 的概念，
    /// 在这里定义一份枚举只会多一处需要同步的映射。
    pub log_level: String,
    /// 缺省帧率（新建任务时的初值）。用 Rational 的字符串形式存，
    /// 因为 10 §1 要求精确帧率，存 f64 会把 24000/1001 变成 23.976023976…。
    pub default_fps: String,
    /// 缺省画布，形如 `1920x1080`。
    pub default_canvas: String,
    /// 1.0.0 之前的临时开关放进这里，不污染上面的稳定字段。
    pub experimental: BTreeMap<String, bool>,
    /// 不认识的顶层字段原样保留（36 §3）。
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            schema: CURRENT_SCHEMA,
            ui_language: None,
            log_level: "info".to_string(),
            // 与 CLI `render` 的默认值一致（args.rs 的 Fps::NTSC_23976）
            default_fps: "24000/1001".to_string(),
            default_canvas: "1920x1080".to_string(),
            experimental: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 受管字段名。写回时用它区分"我们的字段"与"别人的字段"。
const MANAGED: &[&str] = &[
    "schema",
    "ui_language",
    "log_level",
    "default_fps",
    "default_canvas",
    "experimental",
];

/// 一次读取的结果。
///
/// `problems` 不空**不是**失败：设置退回默认值，程序照常启动，
/// 但用户必须能看到"你的设置没被采纳，因为……"。
#[derive(Clone, Debug)]
pub struct SettingsLoad {
    pub settings: AppSettings,
    /// 文件在哪（`None` = 拿不到数据目录）。
    pub path: Option<PathBuf>,
    /// 得到的值是新写的还是退回的默认值。
    pub source: SettingsSource,
    /// 非空就意味着"有些东西没读进来"，每条都必须能被人读懂。
    pub problems: Vec<String>,
    /// 原文件是否已被改名保留（坏文件的证据）。
    pub quarantined: Option<PathBuf>,
}

/// 值从哪来。报告要报这个，否则用户没法判断"我设的东西生效了没有"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSource {
    /// 文件不存在，用的是内置默认值
    Defaults,
    /// 从设置文件读到的
    File,
    /// 文件存在但读不了/解析不了，已退回默认值
    FallbackAfterError,
}

impl SettingsSource {
    pub fn label(self) -> &'static str {
        match self {
            SettingsSource::Defaults => "defaults",
            SettingsSource::File => "file",
            SettingsSource::FallbackAfterError => "fallback-after-error",
        }
    }
}

/// 设置文件的读写入口。
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// 用给定的数据目录构造。
    pub fn in_dir(dir: &DataDir) -> SettingsStore {
        SettingsStore {
            path: dir.file(FILE_NAME),
        }
    }

    /// 用当前 exe 能解析到的数据目录构造（CLI 与 GUI 的默认路径）。
    pub fn current() -> SettingsStore {
        SettingsStore::in_dir(&DataDir::resolve_current_exe())
    }

    /// 显式指定路径（测试与 `STK_SETTINGS` 用）。
    pub fn at(path: PathBuf) -> SettingsStore {
        SettingsStore { path: Some(path) }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 读设置。**永不失败**：拿不到就用默认值，并把原因放进 `problems`。
    pub fn load(&self) -> SettingsLoad {
        let mut problems = Vec::new();
        let Some(path) = self.path.clone() else {
            problems.push(
                "环境里既没有 STK_DATA_DIR 也没有 LOCALAPPDATA/APPDATA，无法定位设置文件；本次用内置默认值"
                    .to_string(),
            );
            return SettingsLoad {
                settings: AppSettings::default(),
                path: None,
                source: SettingsSource::Defaults,
                problems,
                quarantined: None,
            };
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SettingsLoad {
                    settings: AppSettings::default(),
                    path: Some(path),
                    source: SettingsSource::Defaults,
                    problems,
                    quarantined: None,
                };
            }
            Err(e) => {
                problems.push(format!(
                    "读不到设置文件 {}：{e}；本次用内置默认值",
                    path.display()
                ));
                return SettingsLoad {
                    settings: AppSettings::default(),
                    path: Some(path),
                    source: SettingsSource::FallbackAfterError,
                    problems,
                    quarantined: None,
                };
            }
        };
        match parse(&text) {
            Ok((s, mut warns)) => {
                problems.append(&mut warns);
                let source = SettingsSource::File;
                SettingsLoad {
                    settings: s,
                    path: Some(path),
                    source,
                    problems,
                    quarantined: None,
                }
            }
            Err(e) => {
                // 坏文件先留证据再重建：用户可能想看一眼自己丢的是什么。
                let quarantined = quarantine(&path, &mut problems);
                problems.push(format!(
                    "设置文件解析失败（{e}）；已退回内置默认值{}",
                    match &quarantined {
                        Some(q) => format!("，原文件保留在 {}", q.display()),
                        None => "，原文件无法改名保留".to_string(),
                    }
                ));
                SettingsLoad {
                    settings: AppSettings::default(),
                    path: Some(path),
                    source: SettingsSource::FallbackAfterError,
                    problems,
                    quarantined,
                }
            }
        }
    }

    /// 写设置。原子写：同目录临时文件 + 改名。
    pub fn save(&self, s: &AppSettings) -> Result<PathBuf> {
        let Some(path) = self.path.clone() else {
            return Err(
                StkError::simple(Domain::Input, 310, "没有可写的数据目录，设置无法保存")
                    .with_action("设置 STK_DATA_DIR 指向一个可写目录，或检查 LOCALAPPDATA"),
            );
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                StkError::simple(Domain::Input, 311, "创建设置目录失败")
                    .with_detail(format!("{}：{e}", dir.display()))
            })?;
        }
        let text = self.serialize(s)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes()).map_err(|e| {
            StkError::simple(Domain::Input, 312, "写设置临时文件失败")
                .with_detail(format!("{}：{e}", tmp.display()))
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            StkError::simple(Domain::Input, 313, "改名设置文件失败").with_detail(format!(
                "{} -> {}：{e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(path)
    }

    /// 序列化成 JSON 文本。未知字段合并在内（36 §3）。
    pub fn serialize(&self, s: &AppSettings) -> Result<String> {
        let mut o = serde_json::Map::new();
        // 先放别人的字段，再覆盖我们的：这样即使 old 里混进了同名键，
        // 生效的也一定是当前值（而不是读回来的陈旧副本）。
        for (k, v) in &s.extra {
            if !MANAGED.contains(&k.as_str()) {
                o.insert(k.clone(), v.clone());
            }
        }
        o.insert("schema".into(), serde_json::json!(s.schema));
        match &s.ui_language {
            Some(l) => {
                o.insert("ui_language".into(), serde_json::json!(l));
            }
            None => {
                // 显式的 null 而不是省略键：省略会与"旧版没这个字段"混淆，
                // 而 null 是"用户选了跟随系统"这个明确意图。
                o.insert("ui_language".into(), serde_json::Value::Null);
            }
        }
        o.insert("log_level".into(), serde_json::json!(s.log_level));
        o.insert("default_fps".into(), serde_json::json!(s.default_fps));
        o.insert("default_canvas".into(), serde_json::json!(s.default_canvas));
        let exp: serde_json::Map<String, serde_json::Value> = s
            .experimental
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        o.insert("experimental".into(), serde_json::Value::Object(exp));
        serde_json::to_string_pretty(&serde_json::Value::Object(o)).map_err(|e| {
            StkError::simple(Domain::Input, 314, "设置序列化失败").with_detail(e.to_string())
        })
    }
}

/// 解析。返回 `(设置, 警告)`；`Err` 只在"整个 JSON 读不了"时出现。
///
/// 单个字段类型不对**不算**整体失败：那个字段退回默认值并记警告，
/// 其余字段照常生效。理由与"文件坏了不自伤"一致 —— 设置是偏好，
/// 一个拼错的 `log_level` 不该让整份设置作废。
fn parse(text: &str) -> std::result::Result<(AppSettings, Vec<String>), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let serde_json::Value::Object(obj) = v else {
        return Err("顶层不是 JSON 对象".to_string());
    };
    let mut warns = Vec::new();
    let mut s = AppSettings::default();

    // schema：缺失当 1（第一版）；不是整数就报警并当 1
    match obj.get("schema") {
        None => {}
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(x) if x <= u32::MAX as u64 => s.schema = x as u32,
            _ => warns.push(format!(
                "schema 取值 {n} 不在 u32 范围内，按 {CURRENT_SCHEMA} 处理"
            )),
        },
        Some(other) => warns.push(format!(
            "schema 不是整数（{other}），按 {CURRENT_SCHEMA} 处理"
        )),
    }
    if s.schema > CURRENT_SCHEMA {
        // 只读不写：新版写的设置里可能有旧版无法表达的语义，
        // 强行写回等于降级破坏。这一点由调用方在 save 前判断（见 should_write）。
        warns.push(format!(
            "设置文件的 schema 是 {}，高于本程序支持的 {CURRENT_SCHEMA}：只读不写，避免降级破坏新版字段",
            s.schema
        ));
    }

    match obj.get("ui_language") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(x)) => s.ui_language = Some(x.clone()),
        Some(other) => warns.push(format!("ui_language 不是字符串（{other}），按跟随系统处理")),
    }

    match obj.get("log_level") {
        None => {}
        Some(serde_json::Value::String(x)) => s.log_level = x.clone(),
        Some(other) => warns.push(format!(
            "log_level 不是字符串（{other}），按默认值 {} 处理",
            s.log_level
        )),
    }

    if let Some(v) = obj.get("default_fps") {
        match v.as_str() {
            // 只接受能被 Fps 解析的写法：把非法值放进设置，会在"新建任务"那一刻
            // 才炸，而那时用户已经忘了自己改过设置。
            Some(x) => match crate::time::Fps::from_label(x) {
                Ok(_) => s.default_fps = x.to_string(),
                Err(_) => warns.push(format!(
                    "default_fps 取值 {x:?} 不是合法帧率，按默认值 {} 处理",
                    s.default_fps
                )),
            },
            None => warns.push(format!("default_fps 不是字符串（{v}），按默认值处理")),
        }
    }

    if let Some(v) = obj.get("default_canvas") {
        match v.as_str() {
            Some(x) => match parse_canvas(x) {
                Some(_) => s.default_canvas = x.to_string(),
                None => warns.push(format!(
                    "default_canvas 取值 {x:?} 不是 WxH 形式，按默认值 {} 处理",
                    s.default_canvas
                )),
            },
            None => warns.push(format!("default_canvas 不是字符串（{v}），按默认值处理")),
        }
    }

    if let Some(v) = obj.get("experimental") {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    match val.as_bool() {
                        Some(b) => {
                            s.experimental.insert(k.clone(), b);
                        }
                        None => warns.push(format!("experimental.{k} 不是布尔值（{val}），已忽略")),
                    }
                }
            }
            other => warns.push(format!("experimental 不是对象（{other}），已忽略")),
        }
    }

    // 剩下的原样留着（36 §3）
    for (k, v) in obj {
        if !MANAGED.contains(&k.as_str()) {
            s.extra.insert(k, v);
        }
    }
    Ok((s, warns))
}

/// 把坏设置文件改名保留。返回新路径。
///
/// 时间戳用秒级的 UNIX 秒而不是本地时间格式：这里只需要"不同次损坏不互相覆盖"，
/// 而格式化本地时间要引时区处理，代价与收益不成比例。
fn quarantine(path: &Path, problems: &mut Vec<String>) -> Option<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path.file_name()?.to_string_lossy().to_string();
    let bad = path.with_file_name(format!("{name}.bad-{ts}"));
    match std::fs::rename(path, &bad) {
        Ok(()) => Some(bad),
        Err(e) => {
            problems.push(format!("坏设置文件改名失败（{}）：{e}", bad.display()));
            None
        }
    }
}

/// `WxH` 解析。返回 `(宽, 高)`。
///
/// 这里刻意不直接用 `Canvas`：`Canvas` 是**有限枚举**（09 §），而设置里可能存
/// 一个当前版本不认识的合法尺寸。能判"形式对"就先收下，等到真用它的时候
/// 由 `Canvas` 报"不支持这个画布"——那时错误信息里有具体的任务上下文，比
/// "设置文件第 5 行不合法"有用得多。
fn parse_canvas(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 36 §3：未知字段必须往返保留。
    ///
    /// 少了这条，旧版程序读一份新版写的设置、原样写回时会把新版的字段丢掉，
    /// 用户在新版里的选择就静默消失了 —— 而两边都不会报错。
    #[test]
    fn unknown_fields_survive_a_read_write_roundtrip() {
        let dir = tmp("unknown");
        let p = dir.join(FILE_NAME);
        std::fs::write(
            &p,
            r#"{"schema":1,"log_level":"debug","from_a_newer_version":{"nested":[1,2,3]},"another":true}"#,
        )
        .unwrap();
        let store = SettingsStore::at(p.clone());
        let got = store.load();
        assert_eq!(got.source, SettingsSource::File, "{:?}", got.problems);
        assert_eq!(got.settings.log_level, "debug");
        assert!(got.settings.extra.contains_key("from_a_newer_version"));
        assert!(got.settings.extra.contains_key("another"));

        // 写回（改一个受管字段），别人的字段必须还在
        let mut s = got.settings.clone();
        s.log_level = "warn".to_string();
        store.save(&s).unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("from_a_newer_version"),
            "未知字段被丢了：{text}"
        );
        assert!(text.contains("\"another\""), "{text}");
        assert!(text.contains("\"warn\""), "{text}");
        // 内容确实完整（不是只剩一个键名）
        let again: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(again["from_a_newer_version"]["nested"][2], 3, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 坏文件不能让程序起不来：退回默认值 + 评价 + 原文件留证据。
    #[test]
    fn a_corrupt_file_falls_back_and_keeps_the_evidence() {
        let dir = tmp("corrupt");
        let p = dir.join(FILE_NAME);
        std::fs::write(&p, "{ this is not json").unwrap();
        let got = SettingsStore::at(p.clone()).load();
        assert_eq!(got.source, SettingsSource::FallbackAfterError);
        assert_eq!(got.settings, AppSettings::default());
        assert!(!got.problems.is_empty(), "必须说出发生了什么");
        let q = got.quarantined.expect("坏文件该被改名保留");
        assert!(q.is_file(), "证据不见了：{q:?}");
        assert!(
            q.file_name().unwrap().to_string_lossy().contains("bad-"),
            "{q:?}"
        );
        assert!(!p.exists(), "原路径该已让开，否则下次还会读到它");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单个字段类型不对不该让整份设置作废。
    #[test]
    fn one_bad_field_does_not_discard_the_others() {
        let dir = tmp("onebad");
        let p = dir.join(FILE_NAME);
        std::fs::write(
            &p,
            r#"{"schema":1,"log_level":42,"default_fps":"30000/1001","default_canvas":"1920x1080"}"#,
        )
        .unwrap();
        let got = SettingsStore::at(p).load();
        assert_eq!(got.source, SettingsSource::File);
        assert_eq!(got.settings.log_level, "info", "坏字段该退回默认值");
        assert_eq!(got.settings.default_fps, "30000/1001", "好字段该保留");
        assert_eq!(got.settings.default_canvas, "1920x1080");
        assert_eq!(got.problems.len(), 1, "{:?}", got.problems);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 非法帧率/画布要在读的时候就被挡下，而不是等到"新建任务"那一刻才炸。
    #[test]
    fn invalid_fps_and_canvas_are_rejected_at_read_time() {
        let dir = tmp("badvalue");
        let p = dir.join(FILE_NAME);
        std::fs::write(
            &p,
            r#"{"schema":1,"default_fps":"not-a-rate","default_canvas":"1920*1080"}"#,
        )
        .unwrap();
        let got = SettingsStore::at(p).load();
        assert_eq!(got.settings.default_fps, "24000/1001", "{:?}", got.problems);
        assert_eq!(got.settings.default_canvas, "1920x1080");
        assert_eq!(got.problems.len(), 2, "{:?}", got.problems);
        assert!(got.problems.iter().any(|p| p.contains("default_fps")));
        assert!(got.problems.iter().any(|p| p.contains("default_canvas")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// schema 比当前高：只读不写，并说明原因。
    #[test]
    fn a_newer_schema_is_read_but_flagged_as_read_only() {
        let dir = tmp("newschema");
        let p = dir.join(FILE_NAME);
        std::fs::write(&p, r#"{"schema":99,"log_level":"trace"}"#).unwrap();
        let got = SettingsStore::at(p).load();
        assert_eq!(got.settings.schema, 99);
        assert_eq!(got.settings.log_level, "trace", "照样读得出来");
        assert!(
            got.problems.iter().any(|x| x.contains("只读不写")),
            "{:?}",
            got.problems
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 文件不存在 = 默认值，且**不是**问题（首次运行很正常）。
    #[test]
    fn a_missing_file_is_defaults_without_complaint() {
        let dir = tmp("missing");
        let got = SettingsStore::at(dir.join(FILE_NAME)).load();
        assert_eq!(got.source, SettingsSource::Defaults);
        assert!(got.problems.is_empty(), "{:?}", got.problems);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写设置后必须能读回来（含 None 语言的显式 null 往返）。
    #[test]
    fn a_saved_file_reads_back_identical() {
        let dir = tmp("roundtrip");
        let store = SettingsStore::at(dir.join(FILE_NAME));
        let s = AppSettings {
            log_level: "debug".to_string(),
            default_fps: "25".to_string(),
            experimental: BTreeMap::from([("preview_gpu".to_string(), true)]),
            ..AppSettings::default()
        };
        store.save(&s).unwrap();
        let got = store.load();
        assert_eq!(got.source, SettingsSource::File, "{:?}", got.problems);
        assert_eq!(got.settings, s);
        assert!(got.problems.is_empty(), "{:?}", got.problems);

        // 语言设成具体值也要往返
        let mut s2 = s.clone();
        s2.ui_language = Some("zh-Hans".to_string());
        store.save(&s2).unwrap();
        assert_eq!(
            store.load().settings.ui_language.as_deref(),
            Some("zh-Hans")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写入必须原子：临时文件不能在写完之后留下。
    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = tmp("atomic");
        let p = dir.join(FILE_NAME);
        SettingsStore::at(p.clone())
            .save(&AppSettings::default())
            .unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()], "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 受管字段不会被 `extra` 里的同名键盖住。
    /// 少了这条，一份被手工编辑过的设置可以让"用户改的值"静默失效。
    #[test]
    fn extra_cannot_shadow_managed_fields() {
        let dir = tmp("shadow");
        let mut s = AppSettings {
            log_level: "warn".to_string(),
            ..AppSettings::default()
        };
        s.extra
            .insert("log_level".to_string(), serde_json::json!("error"));
        let store = SettingsStore::at(dir.join(FILE_NAME));
        store.save(&s).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(FILE_NAME)).unwrap()).unwrap();
        assert_eq!(v["log_level"], "warn", "受管字段被 extra 盖住了：{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
