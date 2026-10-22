//! 36 §1 的 **ToolDefaults**：每个工具的默认参数，跨会话保留。
//!
//! ## 它在分层里的位置
//!
//! 36 §1 把配置分成五层：
//!
//! | 层 | 作用 | 状态 |
//! | --- | --- | --- |
//! | AppSettings | 全局偏好（日志级别、界面语言…） | 已实现 |
//! | **ToolDefaults** | **每个工具的默认参数（本模块）** | **本模块** |
//! | Preset | 用户可命名、导入导出（TOML） | 已实现；GUI 入口未实现 |
//! | JobSnapshot | 一次执行的完整冻结参数 | 已实现（JSON batch spec） |
//! | Plan | Planner 生成的 stage DAG | 未实现；当前只有无依赖边的被动阶段列表 |
//!
//! 分工是：**AppSettings 管"两个前端共用的显示/运行偏好"，ToolDefaults 管
//! "某个子命令不写参数时用什么值"**。两者的区别不是重要性，而是**使用范围**：
//! 日志级别全局只有一个，而 `pgs encode` 的默认 `--level` 与 `render` 的
//! 默认 `--min-axis` 各管各的。
//!
//! ## 为什么不做成"把 CLI 参数全存一遍"
//!
//! 那种做法有两个后果，都是坏的：
//!
//! 1. **它会与 CLI 的默认值分叉。** 用户在设置里改过 `pgs.level` 之后，
//!    CLI 的 `--help` 里写的默认值就不再是真的默认值了，而没人会去同步那两处。
//! 2. **它会把每个新增参数都变成一次设置文件的 schema 变更。**
//!    加一个 `--new-flag` 就要考虑"旧的设置文件里没有它怎么办"。
//!
//! 所以这里只登记**明确列在 [`Tool`] 里**的少数项，且每一项都必须说清
//! 它与 CLI 默认值的关系。**没登记的工具就是没有默认值可存** ——
//! 不给"任意键值对"开口子。
//!
//! ## 三条纪律（与 AppSettings 同源，理由见 `settings.rs`）
//!
//! 1. 文件坏了不能让人开不了程序：退回默认值 + 说出失败。
//! 2. 未知字段保留（36 §3）：旧版读新版写的文件、写回时不丢字段。
//! 3. 写入原子 + 坏文件保留证据。
//!
//! 这三条在 `settings.rs` 里有完整推导。本模块复用同一套做法而不是另立一套，
//! 但**不共用那份代码**：`settings.rs` 的 `serialize`/`save` 与
//! `AppSettings` 的字段集是耦合的，抽公共部分会把两个独立的 schema
//! 绑在一起，而它们本来就该能各自演进。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::datadir::DataDir;
use crate::error::{Domain, Result, StkError};

/// 当前 ToolDefaults 文件的 schema 版本。
///
/// 名字带上 `TOOL_DEFAULTS_` 前缀而不叫 `CURRENT_SCHEMA`：
/// `settings.rs` 里已经有一个同名常量，而 `core-model` 的
/// 错误码唯一性测试会把**同名常量**当成“同一个编号”来核对 ——
/// 两个不同文件里各自定义 `CURRENT_SCHEMA` 会让它报“编号无法确定”。
/// 名字区分开既让测试能继续把关，也让读代码的人一眼看出是哪份文件。
pub const TOOL_DEFAULTS_CURRENT_SCHEMA: u32 = 1;

/// 文件名（在数据目录里）。
pub const FILE_NAME: &str = "tool-defaults.json";

/// 一个有默认值可存的工具。
///
/// 取值是 CLI 的子命令名（含空格的两段式），与 `args.rs` 的 `Command::name()`
/// 同源 —— 这样"设置里写的 tool 名"与"报告里写的命令名"是同一个字符串，
/// 用户能把两处对上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    PgsEncode,
    Render,
    FontSubset,
    Mkv,
}

impl Tool {
    /// 与 `Command::name()` 一致的字符串。
    pub fn name(self) -> &'static str {
        match self {
            Self::PgsEncode => "pgs encode",
            Self::Render => "render",
            Self::FontSubset => "font subset",
            Self::Mkv => "mkv",
        }
    }

    /// 这个工具**允许**存的键，以及每个键的一句话说明。
    ///
    /// 白名单而不是"什么都收"：一个收任意键的设置文件会在拼错时静默无效
    /// （用户写 `levle` 而我们当它是"别的版本的字段"保留下来，然后不生效）。
    /// 白名单能让拼错变成一条明确的警告。
    pub fn keys(self) -> &'static [(&'static str, &'static str)] {
        match self {
            // 14 §ASS→SUP 的默认；与 args.rs 里 pgs encode 的默认一一对应
            Self::PgsEncode => &[
                ("level", "校验级别：probe|structure|basic|standard|strict"),
                ("canvas", "逻辑画布，形如 1920x1080"),
                ("fps", "帧率，精确有理数形式如 24000/1001"),
                ("default_font", "缺省字体族名"),
                (
                    "exact_colors",
                    "true = Basic（超色即失败），false = Advanced 量化",
                ),
            ],
            Self::Render => &[
                ("canvas", "逻辑画布，形如 1920x1080"),
                ("fps", "帧率，精确有理数形式如 24000/1001"),
                ("default_font", "缺省字体族名"),
                ("min_axis", "PNG 最小边长；0 = 不过滤"),
            ],
            Self::FontSubset => &[
                (
                    "subset_dir",
                    "子集输出目录（绝对路径；留空 = 源字体目录下）",
                ),
                ("fallback_full", "true = 任一份回退就整体失败，不混合交付"),
            ],
            Self::Mkv => &[("tools_dir", "MKVToolNix 安装目录（绝对路径）")],
        }
    }

    pub fn from_name(s: &str) -> Option<Tool> {
        [Tool::PgsEncode, Tool::Render, Tool::FontSubset, Tool::Mkv]
            .into_iter()
            .find(|t| t.name() == s)
    }

    pub const ALL: &'static [Tool] = &[Tool::PgsEncode, Tool::Render, Tool::FontSubset, Tool::Mkv];
}

/// 36 §1 的 ToolDefaults。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDefaults {
    pub schema: u32,
    /// 工具名 → {键 → 值}。值是 JSON 标量或字符串。
    ///
    /// 用 `BTreeMap` 不用 `HashMap`：写回时的字段顺序必须稳定，
    /// 否则同一份设置每次保存都会产生一个"内容相同但字节不同"的文件，
    /// 让 diff 与哈希比较失去意义。
    pub tools: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// 不认识的顶层字段原样保留（36 §3）。
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for ToolDefaults {
    fn default() -> Self {
        ToolDefaults {
            schema: TOOL_DEFAULTS_CURRENT_SCHEMA,
            tools: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 值从哪来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultsSource {
    Defaults,
    File,
}

impl DefaultsSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Defaults => "内置默认值",
            Self::File => "设置文件",
        }
    }
}

/// 一次读取的结果。语义与 `SettingsLoad` 一致。
#[derive(Clone, Debug)]
pub struct DefaultsLoad {
    pub defaults: ToolDefaults,
    pub path: Option<PathBuf>,
    pub source: DefaultsSource,
    /// 非空 = 有些东西没读进来；每条都要能被人读懂。
    pub problems: Vec<String>,
    pub quarantined: Option<PathBuf>,
}

/// 对某个工具的一次求值结果：它**实际会用的**取值。
///
/// 这个结构存在的理由：用户问的是"我设的生效了吗"。只报"设置文件里有这一条"
/// 不回答问题 —— 那一项可能因为拼错键名而不被采纳。所以要报**最终取值**
/// 加**它是从哪来的**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub tool: &'static str,
    /// 最终生效的取值（按键排序）。
    pub effective: BTreeMap<String, String>,
    /// 文件里有、但不在该工具白名单里的键。**报出来而不是丢掉** ——
    /// 拼错键名是最常见的用法错误，静默忽略会让用户以为设置坏了。
    pub unknown_keys: Vec<String>,
}

/// 求值：给定工具，算出实际生效的取值。
pub fn resolve(d: &ToolDefaults, tool: Tool) -> Resolved {
    let mut effective = BTreeMap::new();
    let mut unknown_keys = Vec::new();
    let allowed: Vec<&str> = tool.keys().iter().map(|(k, _)| *k).collect();

    if let Some(m) = d.tools.get(tool.name()) {
        for (k, v) in m {
            if allowed.contains(&k.as_str()) {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                effective.insert(k.clone(), s);
            } else {
                unknown_keys.push(k.clone());
            }
        }
    }
    // 未设置的键不出现 —— 调用方据此沿用 CLI 自己的默认值。
    // **不在这里补 CLI 的默认值**：那会造出第二份默认值定义，
    // 而两份默认值定义分叉的后果正是本模块开头写的那一条。
    Resolved {
        tool: tool.name(),
        effective,
        unknown_keys,
    }
}

/// 读写 ToolDefaults 的地方。
#[derive(Clone, Debug)]
pub struct DefaultsStore {
    path: Option<PathBuf>,
}

impl DefaultsStore {
    /// `DataDir::file` 本身就可能返回 `None`（拿不到数据目录）。
    /// 直接把那个 `None` 传下去，而不是在这里 `unwrap` ——
    /// 拿不到数据目录是一个**正常**情形（没 HOME 的环境），
    /// 它应当让读写分别变成“用默认值”与“明确报错”，而不是 panic。
    pub fn in_dir(dir: &DataDir) -> DefaultsStore {
        DefaultsStore {
            path: dir.file(FILE_NAME),
        }
    }

    /// 当前进程该用哪一份：`STK_TOOL_DEFAULTS` 优先，否则数据目录。
    pub fn current() -> DefaultsStore {
        if let Some(p) = std::env::var_os("STK_TOOL_DEFAULTS") {
            return DefaultsStore::at(PathBuf::from(p));
        }
        DefaultsStore::in_dir(&DataDir::resolve_current_exe())
    }

    pub fn at(path: PathBuf) -> DefaultsStore {
        DefaultsStore { path: Some(path) }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn load(&self) -> DefaultsLoad {
        let Some(path) = self.path.as_ref() else {
            return DefaultsLoad {
                defaults: ToolDefaults::default(),
                path: None,
                source: DefaultsSource::Defaults,
                problems: vec!["拿不到数据目录；工具默认值用内置默认值".to_string()],
                quarantined: None,
            };
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return DefaultsLoad {
                    defaults: ToolDefaults::default(),
                    path: Some(path.clone()),
                    source: DefaultsSource::Defaults,
                    problems: Vec::new(),
                    quarantined: None,
                };
            }
            Err(e) => {
                return DefaultsLoad {
                    defaults: ToolDefaults::default(),
                    path: Some(path.clone()),
                    source: DefaultsSource::Defaults,
                    problems: vec![format!("读不了 {}：{e}", path.display())],
                    quarantined: None,
                };
            }
        };
        match parse(&text) {
            Ok((d, problems, schema_too_new)) => {
                // 只向前迁移：高于本程序的 schema 只读不写。
                let quarantined = None;
                let mut problems = problems;
                if schema_too_new {
                    problems.push(format!(
                        "schema {} 高于本程序支持的 {}；将只读不写，避免降级破坏新版字段",
                        d.schema, TOOL_DEFAULTS_CURRENT_SCHEMA
                    ));
                }
                DefaultsLoad {
                    defaults: d,
                    path: Some(path.clone()),
                    source: DefaultsSource::File,
                    problems,
                    quarantined,
                }
            }
            Err(why) => {
                // 坏文件：改名保留证据，退回默认值。
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let bad = path.with_extension(format!("json.bad-{stamp}"));
                let quarantined = std::fs::rename(path, &bad).ok().map(|_| bad);
                DefaultsLoad {
                    defaults: ToolDefaults::default(),
                    path: Some(path.clone()),
                    source: DefaultsSource::Defaults,
                    problems: vec![format!("{why}；已退回内置默认值")],
                    quarantined,
                }
            }
        }
    }

    /// 原子写入：同目录临时文件 + 改名。
    pub fn save(&self, d: &ToolDefaults) -> Result<PathBuf> {
        let Some(path) = self.path.as_ref() else {
            return Err(StkError::simple(
                Domain::Input,
                213,
                "拿不到数据目录，无法保存工具默认值",
            ));
        };
        let text = serialize(d)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StkError::simple(Domain::Output, 214, "创建数据目录失败").with_detail(e.to_string())
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(|e| {
            StkError::simple(Domain::Output, 214, "写临时文件失败").with_detail(e.to_string())
        })?;
        std::fs::rename(&tmp, path).map_err(|e| {
            StkError::simple(Domain::Output, 214, "改名到目标失败").with_detail(e.to_string())
        })?;
        Ok(path.clone())
    }
}

/// 解析。返回 (值, 警告, schema 是否高于本程序)。
///
/// **未知字段进 `extra`，不报错** —— 36 §3 的往返要求。
/// **未知的键/值类型产生警告，但不废掉整份文件**。
fn parse(text: &str) -> std::result::Result<(ToolDefaults, Vec<String>, bool), String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("工具默认值文件不是合法 JSON：{e}"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| "工具默认值文件的顶层必须是对象".to_string())?;

    let mut problems = Vec::new();
    let mut out = ToolDefaults::default();

    let schema = match obj.get("schema").and_then(|x| x.as_u64()) {
        Some(n) => u32::try_from(n).unwrap_or(0),
        None => {
            problems.push("缺少 schema 字段，按当前版本解析".to_string());
            TOOL_DEFAULTS_CURRENT_SCHEMA
        }
    };
    out.schema = schema;
    let schema_too_new = schema > TOOL_DEFAULTS_CURRENT_SCHEMA;

    for (k, val) in obj {
        match k.as_str() {
            "schema" => continue,
            "tools" => {
                let Some(m) = val.as_object() else {
                    problems.push("tools 必须是对象；已忽略".to_string());
                    continue;
                };
                for (tool_name, tool_val) in m {
                    let Some(tm) = tool_val.as_object() else {
                        problems.push(format!("tools.{tool_name} 必须是对象；已忽略"));
                        continue;
                    };
                    // 工具名不认识：还是收着（可能是新版加的工具），
                    // 但明确说一句 —— 静默收着会让"我设了却不生效"变成找不到原因的事。
                    if Tool::from_name(tool_name).is_none() {
                        problems.push(format!(
                            "不认识工具「{tool_name}」（已知：{}）；已保留但不参与执行",
                            Tool::ALL
                                .iter()
                                .map(|t| t.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    let mut entry = BTreeMap::new();
                    for (kk, vv) in tm {
                        if !vv.is_string() && !vv.is_boolean() && !vv.is_number() {
                            problems.push(format!(
                                "tools.{tool_name}.{kk} 的类型不支持（只收字符串/布尔/数字）；已忽略"
                            ));
                            continue;
                        }
                        entry.insert(kk.clone(), vv.clone());
                    }
                    out.tools.insert(tool_name.clone(), entry);
                }
            }
            _ => {
                out.extra.insert(k.clone(), val.clone());
            }
        }
    }
    Ok((out, problems, schema_too_new))
}

/// 序列化。写回时把 `extra` 合并回去（36 §3 的往返）。
pub fn serialize(d: &ToolDefaults) -> Result<String> {
    let mut obj = serde_json::Map::new();
    obj.insert("schema".into(), serde_json::json!(d.schema));
    let mut tools = serde_json::Map::new();
    for (k, m) in &d.tools {
        let mut inner = serde_json::Map::new();
        for (kk, vv) in m {
            inner.insert(kk.clone(), vv.clone());
        }
        tools.insert(k.clone(), serde_json::Value::Object(inner));
    }
    obj.insert("tools".into(), serde_json::Value::Object(tools));
    // 别人的字段原样放回
    for (k, v) in &d.extra {
        obj.insert(k.clone(), v.clone());
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map(|s| s + "\n")
        .map_err(|e| {
            StkError::simple(Domain::Output, 215, "工具默认值序列化失败").with_detail(e.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-td-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_absent_file_is_not_a_problem() {
        let d = tmp("absent");
        let l = DefaultsStore::at(d.join("nope.json")).load();
        assert_eq!(l.source, DefaultsSource::Defaults);
        assert!(
            l.problems.is_empty(),
            "文件不存在不是问题: {:?}",
            l.problems
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_round_trip_preserves_everything_we_set() {
        let d = tmp("rt");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        let mut td = ToolDefaults::default();
        let mut m = BTreeMap::new();
        m.insert("level".to_string(), serde_json::json!("standard"));
        m.insert("canvas".to_string(), serde_json::json!("1920x1080"));
        td.tools.insert("pgs encode".into(), m);

        store.save(&td).unwrap();
        let back = store.load();
        assert_eq!(back.source, DefaultsSource::File);
        assert!(back.problems.is_empty(), "{:?}", back.problems);
        assert_eq!(back.defaults.tools, td.tools);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_top_level_fields_survive_a_round_trip() {
        // 36 §3：旧版读新版写的文件、写回时不丢字段。
        let d = tmp("fwd");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        std::fs::write(
            store.path().unwrap(),
            r#"{"schema":1,"tools":{},"future_thing":{"a":1}}"#,
        )
        .unwrap();
        let l = store.load();
        assert_eq!(
            l.defaults.extra.get("future_thing"),
            Some(&serde_json::json!({"a": 1}))
        );
        store.save(&l.defaults).unwrap();
        let raw = std::fs::read_to_string(store.path().unwrap()).unwrap();
        assert!(raw.contains("future_thing"), "写回时丢了未知字段: {raw}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_file_is_quarantined_and_defaults_are_used() {
        let d = tmp("bad");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        std::fs::write(store.path().unwrap(), "{ this is not json").unwrap();
        let l = store.load();
        assert_eq!(l.source, DefaultsSource::Defaults, "坏文件要退回默认值");
        assert!(!l.problems.is_empty(), "坏文件必须说出来");
        let q = l.quarantined.expect("坏文件要留证据");
        assert!(q.exists(), "留证据的那个文件不存在");
        assert!(!store.path().unwrap().exists(), "原文件应当已被改名");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_schema_newer_than_ours_is_reported_not_silently_interpreted() {
        let d = tmp("new");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        std::fs::write(store.path().unwrap(), r#"{"schema":99,"tools":{}}"#).unwrap();
        let l = store.load();
        assert!(
            l.problems.iter().any(|p| p.contains("只读不写")),
            "高于本程序的 schema 必须说明: {:?}",
            l.problems
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_only_returns_whitelisted_keys() {
        let mut td = ToolDefaults::default();
        let mut m = BTreeMap::new();
        m.insert("level".to_string(), serde_json::json!("standard"));
        m.insert("levle".to_string(), serde_json::json!("typo"));
        td.tools.insert("pgs encode".into(), m);

        let r = resolve(&td, Tool::PgsEncode);
        assert_eq!(r.effective.get("level"), Some(&"standard".to_string()));
        assert!(!r.effective.contains_key("levle"), "拼错的键不该生效");
        assert_eq!(
            r.unknown_keys,
            vec!["levle".to_string()],
            "拼错的键要报出来"
        );
    }

    #[test]
    fn resolve_does_not_invent_cli_defaults() {
        // 没设过的键**不出现**：补上 CLI 的默认值会造出第二份默认值定义。
        let td = ToolDefaults::default();
        let r = resolve(&td, Tool::PgsEncode);
        assert!(r.effective.is_empty(), "空设置不该凭空生出取值");
    }

    #[test]
    fn every_tool_name_matches_the_cli_command_name() {
        // 设置里写的工具名必须与报告里的命令名是同一个字符串，
        // 否则用户没法把两处对上。
        for t in Tool::ALL {
            assert!(!t.name().is_empty());
            assert_eq!(Tool::from_name(t.name()), Some(*t));
        }
        assert_eq!(Tool::from_name("nope"), None);
    }

    #[test]
    fn serialization_is_stable_across_runs() {
        // 同一份内容两次序列化必须逐字节相同 —— 否则每次保存都产生
        // 一个"内容相同但字节不同"的文件，diff 与哈希比较失去意义。
        let mut td = ToolDefaults::default();
        let mut m = BTreeMap::new();
        m.insert("b".to_string(), serde_json::json!(2));
        m.insert("a".to_string(), serde_json::json!(1));
        td.tools.insert("render".into(), m);
        assert_eq!(serialize(&td).unwrap(), serialize(&td).unwrap());
    }

    #[test]
    fn an_unknown_tool_is_kept_but_reported() {
        let d = tmp("unk");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        std::fs::write(
            store.path().unwrap(),
            r#"{"schema":1,"tools":{"future tool":{"x":1}}}"#,
        )
        .unwrap();
        let l = store.load();
        assert!(l.defaults.tools.contains_key("future tool"), "要保留");
        assert!(
            l.problems.iter().any(|p| p.contains("future tool")),
            "要报出来: {:?}",
            l.problems
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unsupported_value_type_is_reported_not_stored() {
        let d = tmp("type");
        let store = DefaultsStore::at(d.join(FILE_NAME));
        std::fs::write(
            store.path().unwrap(),
            r#"{"schema":1,"tools":{"render":{"canvas":[1,2]}}}"#,
        )
        .unwrap();
        let l = store.load();
        assert!(
            l.problems.iter().any(|p| p.contains("类型不支持")),
            "{:?}",
            l.problems
        );
        assert!(!l.defaults.tools["render"].contains_key("canvas"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saving_without_a_data_dir_is_an_error_not_a_silent_no_op() {
        let store = DefaultsStore { path: None };
        let e = store.save(&ToolDefaults::default()).unwrap_err();
        assert!(e.title.contains("拿不到数据目录"), "{}", e.title);
    }
}
