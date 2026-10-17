//! 字体数据库（17 §2）。
//!
//! 17 §2 明说"不能只扫描 `C:\\Windows\\Fonts` 的文件名"，因为注册名、本地化名、TTC、
//! 用户字体与可变字体都会让"文件名 == Fontname"失效。这个模块的立场是：**读表，不猜**。
//! 每个 face 的家族名/子家族/全名/PostScript 名、以及各平台的本地化名都从 `name` 表里取，
//! 字重与斜体从 `OS/2` 取，覆盖从 `cmap` 取，可变性看 `fvar`。
//!
//! 解析器选择：`skrifa` + `read-fonts`（fontations / Google Fonts 一系，Apache-2.0 OR MIT）。
//! 早先用的是 `ttf-parser`，被 RustSec 判为 unmaintained（RUSTSEC-2026-0192，
//! "No safe upgrade is available!"）——而这里的输入是**任意用户字体文件与 ASS 内嵌字体**，
//! 不可信字节需要一个还在收修复的解析器。`deny.toml` 的 `[advisories] ignore` 是故意留空的，
//! 所以走迁移而不是豁免（ISSUES AG15）。
//!
//! 已知边界（都写进 ISSUES AG，不当成已完成）：
//! * 17 §2 的权威是 DirectWrite。这里做的是"文件系统 + 表"的视角，看不到
//!   用户注册字体（`HKCU\...\Fonts`）与 DirectWrite 的 fallback 链（AG1）。
//! * 覆盖度是 cmap 级的："这个 face 有没有这个码位"。形状是否可用（复杂脚本的 GSUB
//!   替换、组合记号的 GPOS 定位）不在这一层判断。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use core_model::error::{Domain, Result, StkError};
use read_fonts::tables::os2::SelectionFlags;
use read_fonts::{FileRef, FontRef, TableProvider};
use sha2::{Digest as _, Sha256};
use skrifa::charmap::Charmap;
use skrifa::string::StringId;
use skrifa::MetadataProvider;

/// 能读的文件扩展名。`woff`/`woff2` 是压缩容器，本层不解析，扫到就跳过并记一条说明。
pub const FONT_EXTS: &[&str] = &["ttf", "otf", "ttc", "otc"];

/// `fvar` 里的一条可变轴（17 §2 的 "variable axes"；AG9 登记缺的就是这个）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisInfo {
    /// 四字节标签，如 `wght`、`wdth`、`opsz`
    pub tag: String,
    /// 三个值都是设计单位（user units）。这里用 f32 不是做近似计算，只是原样搬运表值，
    /// 所以本结构不参与判等之外的任何数值运算。
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// 一个 face（TTC/OTC 里的一个 face 就是一条）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FaceRecord {
    pub file: PathBuf,
    pub face_index: u32,
    /// 这个文件里一共有多少个 face；`face_index > 0` 说明是集合成员
    pub collection_len: u32,
    /// typographic family（name ID 16），没有就退回 ID 1
    pub family: String,
    /// Win32 family（ID 1）：很多老程序实际匹配的是它
    pub win32_family: String,
    pub full_name: String,
    pub postscript_name: String,
    /// 子家族（ID 17 优先，退回 ID 2）
    pub style_name: String,
    /// 所有平台/语言下的家族名（含中文本地化名，如"微软雅黑"）
    pub family_aliases: Vec<String>,
    /// `OS/2.usWeightClass`：1…1000；常见等级为 100 的倍数，400 常规、700 粗体。
    pub weight: u16,
    pub italic: bool,
    /// `OS/2.usWidthClass`：1…9，**5 才是常规**。
    ///
    /// 不是百分比。这里的注释曾经写成"100 极窄 … 200 极宽，125 为常规"，而存进去的一直是
    /// 1…9 的等级值，于是评分层拿 100 当基准——越宽反而越"接近常规"。
    /// report.rs 的 `score` 现在按 5 计罚分，这条注释按实际字段语义写。
    pub width: u16,
    pub variable: bool,
    /// 可变字体才有内容；非可变字体是空表
    pub axes: Vec<AxisInfo>,
    pub num_glyphs: u16,
    pub units_per_em: u16,
    /// 字体文件内容的 SHA-256。17 §2 要的就是这个：换字体必须看得出来
    pub sha256: String,
    pub mtime_secs: Option<u64>,
}

impl FaceRecord {
    /// 请求名能不能对上这条 face：家族、Win32 家族、PostScript、全名、任一本地化名。
    pub fn matches_name(&self, want: &str) -> NameMatch {
        let want = normalize(want);
        if want.is_empty() {
            return NameMatch::No;
        }
        if normalize(&self.family) == want {
            return NameMatch::Family;
        }
        if normalize(&self.win32_family) == want {
            return NameMatch::Win32Family;
        }
        if normalize(&self.postscript_name) == want {
            return NameMatch::PostScript;
        }
        if normalize(&self.full_name) == want {
            return NameMatch::FullName;
        }
        if self.family_aliases.iter().any(|a| normalize(a) == want) {
            return NameMatch::Localized;
        }
        NameMatch::No
    }

    /// 展示用的"文件#face"标识；报告里必须能反查到具体是哪个集合成员。
    pub fn label(&self) -> String {
        if self.collection_len > 1 {
            format!("{}#{}", file_name(&self.file), self.face_index)
        } else {
            file_name(&self.file)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameMatch {
    Family,
    Win32Family,
    PostScript,
    FullName,
    /// 本地化名（中文 Windows 上的"微软雅黑"这类）
    Localized,
    No,
}

/// 扫描过程中的"没读成"记录：不假装没有发生。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanNote {
    pub path: String,
    pub reason: String,
}

/// `missing_glyphs` 发现"索引说哈希是 A，文件现在明明是 B"时报的错误码。
/// 使用点复核就靠这一个值认路（`report::evaluate_or_rescan` 据此决定要不要重来一次），
/// 所以它是有契约意义的，不能随手改数字。
pub const FONT_CHANGED_AFTER_SCAN: u16 = 222;

/// 磁盘缓存怎么用。三个状态各有明确的调用者，不该压成一个 bool。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    /// 指纹相符就吃缓存，否则全量解析并写回。
    Use,
    /// 无条件全量解析并写回。调用者是“使用点完整性复核发现索引已过期后的重扫一次”。
    Refresh,
    /// 完全不碰磁盘缓存，也不写。`--no-font-cache` 用它：想确认"没有缓存时答案是什么"就靠这条。
    Off,
}

/// 这次这份库到底是怎么来的。报告必须能区分"真的重扫过"与"吃了缓存"，否则读者看不出
/// 自己付了哪份代价；同时它是**成本事实**不是环境事实，所以不进渲染身份的摘要
/// （不然"有没有缓存"会让同一个环境产生两个缓存 key，缓存自己把自己打掉）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    /// 全量解析出来的（第一次扫、指纹变了、或这条路径压根不走缓存）
    #[default]
    Scanned,
    /// 命中缓存：索引来自上一次，本次只重读并复核真正被用到的那些 face
    Hit,
    /// 缓存里的索引在使用点被判为过期（内容哈希与文件不符），本次退回全量重扫。
    /// 这是“重建一次索引”，不是“从投毒中恢复”。
    Refreshed,
    /// 调用方明确不要缓存（`--no-font-cache`）
    Disabled,
}

impl CacheState {
    /// 这份索引是不是"上一次读到的东西"。只有它才值得在使用点发现不一致时重扫一次（
    /// 现扫的库就是本次读的字节，再不一致说明文件正在被并发覆盖）。
    pub fn is_hit(self) -> bool {
        self == CacheState::Hit
    }

    /// 报告里那一格。写成短语而不是裸枚举名：读报告的人要知道代价是什么。
    pub fn label(self) -> &'static str {
        match self {
            CacheState::Scanned => "scanned",
            CacheState::Hit => "hit",
            CacheState::Refreshed => "stale-refreshed",
            CacheState::Disabled => "off",
        }
    }

    /// 这一格对读者意味着什么。缓存只能让我们慢或快，不能让我们答错 —— 依据写在这里。
    pub fn meaning(self) -> &'static str {
        match self {
            CacheState::Scanned => "这次把每个字体文件都读了一遍并解析；没有可用的磁盘缓存",
            CacheState::Hit => "索引来自上次扫描，本次只重读了真正被用到的每个 face 的字节",
            CacheState::Refreshed => {
                "索引里的内容哈希与文件现状不符，已退回全量重扫，本次结论与新索引一致"
            }
            CacheState::Disabled => "--no-font-cache：完全没用磁盘缓存，这是最慢也最独立的一条路",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FontDb {
    pub faces: Vec<FaceRecord>,
    pub notes: Vec<ScanNote>,
    /// 扫过的目录，报告里要说清"在哪找过"
    pub roots: Vec<String>,
    pub files_read: usize,
    /// 只来自注册表、目录扫不到的文件数（`scan_registered` 才有意义）
    pub registry_only: usize,
    /// 这份库的来历：命中缓存 / 全量重扫 / 明确不用缓存
    pub cache: CacheState,
}

/// 一个文件最多枚举多少个 face。正常集合文件是个位数；这个上限只用来防
/// "坏文件让扫描停不下来"，不是功能限制。
const MAX_FACES_PER_FILE: usize = 128;

impl FontDb {
    /// 扫描若干目录（**只取一层**，Windows 字体目录是平的；递归会把用户整个仓库读一遍）。
    ///
    /// 顺序按文件名字典序，face 索引升序：同一台机器上两次扫描必须给出同一个数据库，
    /// 否则"字体环境变了"与"目录枚举顺序变了"就分不开（17 §AT-FONT-002 的前提）。
    pub fn scan(dirs: &[&Path]) -> FontDb {
        let mut out = FontDb::default();
        for d in dirs {
            out.roots.push(d.display().to_string());
        }
        let files = out.files_from_dirs(dirs);
        out.absorb(files);
        out
    }

    /// 目录扫描 **加上注册表登记的字体文件**（17 §2 的"字体注册"）。
    ///
    /// 为什么要合起来：只扫 `%WINDIR%\Fonts` 会漏掉装在别的目录却被注册进来的字体，
    /// 而 `evaluate` 只会说"按名字找不到"——那是不存在的结论。注册项解析不到的，
    /// 由调用方把 `RegistryReport` 里的原因并进报告（CLI 的 `registry` 段就是这么做的），
    /// 这里不静默丢。
    ///
    /// 同一个文件可能既被目录扫到、又被注册表指到（本机大多数系统字体都如此），
    /// 所以必须按**规范路径**去重：否则一个 face 会有两条记录，`files_read` 也在虚报。
    pub fn scan_registered(dirs: &[&Path], registered: &[PathBuf]) -> FontDb {
        let mut out = FontDb::default();
        for d in dirs {
            out.roots.push(d.display().to_string());
        }
        if !registered.is_empty() {
            out.roots
                .push("注册表 HKLM/HKCU\\...\\CurrentVersion\\Fonts".to_string());
        }
        let mut files = out.files_from_dirs(dirs);
        let mut seen: BTreeSet<PathBuf> = files.iter().map(|p| dedupe_key(p)).collect();
        let mut via_registry = 0usize;
        for p in registered {
            if !p.is_file() {
                continue;
            }
            let key = dedupe_key(p);
            if seen.insert(key) {
                via_registry += 1;
                files.insert(p.clone());
            }
        }
        out.registry_only = via_registry;
        out.absorb(files);
        out
    }

    /// 目录里一层枚举：字体后缀进集合，已知但不支持的容器留一条说明。
    fn files_from_dirs(&mut self, dirs: &[&Path]) -> BTreeSet<PathBuf> {
        let (files, notes) = list_font_files(dirs);
        self.notes.extend(notes);
        files
    }

    /// 带磁盘缓存的扫描。语义与 `scan`/`scan_registered` 一致，区别只在命中时不重新解析文件。
    ///
    /// 命中时**不做全库复核**：本机实测（154 个文件 / 196 个 face）
    /// release 档一次全量解析 2.69 s，而"把每条记录的文件重读一遍算哈希"按 face 走要 4.91 s、
    /// 按去重后的文件走要 2.63 s；debug 档是 45.4 s 对 83.6 s / 45.0 s。
    /// 也就是说复核整座库的成本 ≈ 重新扫一遍的成本（读文件才是大头，解析几乎不要钱），
    /// 那样一来缓存就没有存在意义了。缓存的可信度不放在这里，放在使用点：见 `report::evaluate_or_rescan`
    /// ——`missing_glyphs` 本来就要重读"真的被用到"的那些 face 来判 cmap，顺手比内容哈希。
    /// 数字与推导都记在 ISSUES AI5。
    pub fn scan_cached(dirs: &[&Path], registered: &[PathBuf], mode: CacheMode) -> FontDb {
        let (mut files, notes) = list_font_files(dirs);
        let mut out = FontDb {
            notes,
            ..Default::default()
        };
        for d in dirs {
            out.roots.push(d.display().to_string());
        }
        let mut via_registry = 0usize;
        let mut seen: BTreeSet<PathBuf> = files.iter().map(|p| dedupe_key(p)).collect();
        for p in registered {
            if !p.is_file() || !has_font_ext(p) {
                continue;
            }
            if seen.insert(dedupe_key(p)) {
                via_registry += 1;
                files.insert(p.clone());
            }
        }
        let mut fp = crate::cache::fingerprint(&files.iter().cloned().collect::<Vec<_>>());
        fp.push(format!("registry={}", registered.len()));
        if mode == CacheMode::Use {
            if let Some(cached) = crate::cache::load(&fp) {
                return crate::cache::to_db(&cached);
            }
        }
        if !registered.is_empty() {
            out.roots
                .push(r"注册表 HKLM/HKCU\...\CurrentVersion\Fonts".to_string());
        }
        out.registry_only = via_registry;
        out.absorb(files);
        out.cache = match mode {
            CacheMode::Off => CacheState::Disabled,
            _ => CacheState::Scanned,
        };
        if mode != CacheMode::Off {
            crate::cache::store(&fp, &out);
        }
        out
    }

    fn absorb(&mut self, files: BTreeSet<PathBuf>) {
        for f in files {
            self.files_read += 1;
            self.read_file(&f);
        }
        self.faces.sort_by(|a, b| {
            (&a.file, a.face_index)
                .partial_cmp(&(&b.file, b.face_index))
                .unwrap()
        });
    }

    fn read_file(&mut self, path: &Path) {
        let Ok(bytes) = std::fs::read(path) else {
            self.notes.push(ScanNote {
                path: path.display().to_string(),
                reason: "文件读不了".to_string(),
            });
            return;
        };
        let sha = digest(&bytes);
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let file = match FileRef::new(&bytes) {
            Ok(f) => f,
            Err(e) => {
                self.notes.push(ScanNote {
                    path: path.display().to_string(),
                    reason: format!("不是可解析的字体或字体集合：{e:?}"),
                });
                return;
            }
        };
        let start = self.faces.len();
        let mut parsed = 0u32;
        for (index, font) in file.fonts().enumerate() {
            if index >= MAX_FACES_PER_FILE {
                self.notes.push(ScanNote {
                    path: path.display().to_string(),
                    reason: format!("face 数超过 {MAX_FACES_PER_FILE}，后面的没有读"),
                });
                break;
            }
            match font {
                Ok(font) => {
                    self.faces
                        .push(record_of(&font, path, index as u32, &sha, mtime));
                    parsed += 1;
                }
                Err(e) => {
                    // 集合里单个成员坏掉不能连坐整个文件，但必须留痕
                    self.notes.push(ScanNote {
                        path: format!("{}#{index}", path.display()),
                        reason: format!("集合成员解析失败：{e:?}"),
                    });
                }
            }
        }
        if parsed == 0 {
            self.notes.push(ScanNote {
                path: path.display().to_string(),
                reason: "文件里一个 face 都没有（空集合或损坏的表目录）".to_string(),
            });
        }
        // collection_len 是"这个文件里有几个 face"：单 face 文件恒为 1，
        // label() 靠它决定要不要带 #索引，所以必须由文件级统计回填
        for rec in self.faces[start..].iter_mut() {
            rec.collection_len = parsed;
        }
    }

    pub fn by_name<'a>(&'a self, want: &'a str) -> impl Iterator<Item = (usize, &'a FaceRecord)> {
        self.faces
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.matches_name(want) != NameMatch::No)
    }

    /// cmap 级的覆盖检查：返回**缺失**的码位（排序去重）。
    ///
    /// 每次核验都重读内容并核哈希：字体文件可能在扫描之后被换掉（升级、覆盖安装、注销），
    /// 哈希变了就不给结论，而不是给一个过期结论（AT-FONT-002）。
    pub fn missing_glyphs(&self, idx: usize, chars: &BTreeSet<char>) -> Result<Vec<char>> {
        let rec = self.faces.get(idx).ok_or_else(|| {
            StkError::simple(Domain::Input, 220, "字体索引不存在").with_detail(format!("{idx}"))
        })?;
        let bytes = std::fs::read(&rec.file).map_err(|e| {
            StkError::simple(Domain::Input, 221, "读不到字体文件，无法判定覆盖")
                .with_detail(format!("{}：{e}", rec.file.display()))
        })?;
        if digest(&bytes) != rec.sha256 {
            return Err(StkError::simple(
                Domain::Input,
                FONT_CHANGED_AFTER_SCAN,
                "字体内容在扫描之后变了",
            )
            .with_detail(format!("{}：索引 {idx}", rec.file.display()))
            .with_action("重跑 font scan 建立新的字体数据库"));
        }
        let font = face_of(&bytes, rec.face_index, &rec.file)?;
        let charmap = font.charmap();
        Ok(chars
            .iter()
            .copied()
            .filter(|c| !covers(&charmap, *c))
            .collect())
    }
}

/// 从文件内容里取第 `index` 个 face。单 face 文件与集合文件走同一条路：
/// `FileRef::fonts()` 对两者都给出正确的序列。
fn face_of<'a>(bytes: &'a [u8], index: u32, file: &Path) -> Result<FontRef<'a>> {
    let fail = |why: String| {
        StkError::simple(Domain::Input, 223, "字体重新解析失败")
            .with_detail(format!("{}：{why}", file.display()))
    };
    let f = FileRef::new(bytes).map_err(|e| fail(format!("{e:?}")))?;
    f.fonts()
        .nth(index as usize)
        .ok_or_else(|| fail(format!("集合里没有第 {index} 个 face")))?
        .map_err(|e| fail(format!("{e:?}")))
}

/// 这份字节里第 0 个 face 的内部家族名（17 §7 的附件注入要用它注册给 libass）。
///
/// 为什么非要内部名：实测 libass 匹配内存字体看的是字体表里的名字，不是 `[Fonts]` 里写的
/// 附件名，也不是文件名（ISSUES AK1）。typographic family（ID 16）优先，退回 Win32 ID 1 ——
/// 与建库时同一个优先级，两条路不能各说一套。
pub fn family_of_bytes(bytes: &[u8]) -> Result<String> {
    let font = face_of(bytes, 0, Path::new("<memory>"))?;
    let mut sink = BTreeSet::new();
    let typo = first_string(&font, StringId::TYPOGRAPHIC_FAMILY_NAME, &mut sink);
    let win = first_string(&font, StringId::FAMILY_NAME, &mut sink);
    let name = if typo.is_empty() { win } else { typo };
    if name.is_empty() {
        return Err(StkError::simple(Domain::Input, 223, "字体重新解析失败")
            .with_detail("<memory>：这份字体没有任何家族名"));
    }
    Ok(name)
}

fn record_of(
    font: &FontRef<'_>,
    path: &Path,
    index: u32,
    sha: &str,
    mtime: Option<u64>,
) -> FaceRecord {
    let mut localized: BTreeSet<String> = BTreeSet::new();
    let family_win = first_string(font, StringId::FAMILY_NAME, &mut localized);
    let family_typo = first_string(
        font,
        StringId::TYPOGRAPHIC_FAMILY_NAME,
        &mut BTreeSet::new(),
    );
    let sub_win = first_string(font, StringId::SUBFAMILY_NAME, &mut BTreeSet::new());
    let sub_typo = first_string(
        font,
        StringId::TYPOGRAPHIC_SUBFAMILY_NAME,
        &mut BTreeSet::new(),
    );
    let full = first_string(font, StringId::FULL_NAME, &mut BTreeSet::new());
    let ps = first_string(font, StringId::POSTSCRIPT_NAME, &mut BTreeSet::new());
    let family = if family_typo.is_empty() {
        family_win.clone()
    } else {
        family_typo
    };
    let style_name = if sub_typo.is_empty() {
        sub_win
    } else {
        sub_typo
    };

    // OS/2 的 usWeightClass/usWidthClass/fsSelection 优先；没有 OS/2 的老字体才看
    // head.macStyle（17 §2 要的 weight/style/stretch 就是这么定的）
    let os2 = font.os2().ok();
    let sel = os2.as_ref().map(|o| o.fs_selection());
    let mac_style = font
        .head()
        .ok()
        .map(|h| h.mac_style().bits())
        .unwrap_or_default();
    let weight = match &os2 {
        Some(o) => o.us_weight_class(),
        None => {
            if mac_style & 0x1 != 0 {
                700
            } else {
                400
            }
        }
    };
    let width = match &os2 {
        Some(o) => o.us_width_class(),
        None => 5,
    };
    let italic = match sel {
        Some(f) => f.contains(SelectionFlags::ITALIC),
        None => mac_style & 0x2 != 0,
    };
    let axes: Vec<AxisInfo> = font
        .axes()
        .iter()
        .map(|a| AxisInfo {
            tag: a.tag().to_string(),
            min: a.min_value(),
            default: a.default_value(),
            max: a.max_value(),
        })
        .collect();
    FaceRecord {
        file: path.to_path_buf(),
        face_index: index,
        collection_len: 1,
        family,
        win32_family: family_win,
        full_name: full,
        postscript_name: ps,
        style_name,
        family_aliases: localized.into_iter().collect(),
        weight,
        italic,
        width,
        variable: !axes.is_empty(),
        axes,
        num_glyphs: font.maxp().map(|m| m.num_glyphs()).unwrap_or(0),
        units_per_em: font.head().map(|h| h.units_per_em()).unwrap_or(0),
        sha256: sha.to_string(),
        mtime_secs: mtime,
    }
}

/// 取某个 name ID 的第一条非空记录；顺带把每条本地化文本收进 `sink`。
fn first_string(font: &FontRef<'_>, id: StringId, sink: &mut BTreeSet<String>) -> String {
    let mut first = String::new();
    for s in font.localized_strings(id) {
        let text = s.to_string().trim().to_string();
        if text.is_empty() {
            continue;
        }
        sink.insert(text.clone());
        if first.is_empty() {
            first = text;
        }
    }
    first
}

/// cmap 覆盖判定。`glyf`/`CFF` 之外的细节不在这里管。
fn covers(charmap: &Charmap<'_>, c: char) -> bool {
    if c == ' ' || c == '\t' {
        // 空白由渲染器处理，不该因为 cmap 没有空格就报缺字
        return true;
    }
    charmap.map(c).is_some()
}

/// 这个后缀是不是本层认的字体文件。
fn has_font_ext(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .map(|s| FONT_EXTS.contains(&s.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 目录一层枚举的纯函数版本：返回（字体文件集合，"没读成"的记录）。
/// `scan` 与 `scan_cached` 共用，两条路对同一个目录必须给出同一份清单。
fn list_font_files(dirs: &[&Path]) -> (BTreeSet<PathBuf>, Vec<ScanNote>) {
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    let mut notes: Vec<ScanNote> = Vec::new();
    for d in dirs {
        let entries = match std::fs::read_dir(d) {
            Ok(e) => e,
            Err(e) => {
                notes.push(ScanNote {
                    path: d.display().to_string(),
                    reason: format!("目录读不了：{e}"),
                });
                continue;
            }
        };
        let _ = &entries;
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if FONT_EXTS.contains(&ext.as_str()) {
                files.insert(p);
            } else if matches!(ext.as_str(), "woff" | "woff2" | "dfont" | "fon") {
                notes.push(ScanNote {
                    path: p.display().to_string(),
                    reason: "这种容器/格式本层不解析（woff/woff2 需要解压，fon 不是 TrueType）"
                        .to_string(),
                });
            }
        }
    }
    (files, notes)
}

/// 去重用的键：同一个文件可能被目录与注册表以不同写法指到（正斜杠/短名/大小写），
/// 能规范化的就用规范化结果；规范化失败就退回原路径——宁可重一条也不能漏一条。
fn dedupe_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

fn file_name(p: &Path) -> String {
    match p.file_name().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => p.display().to_string(),
    }
}
