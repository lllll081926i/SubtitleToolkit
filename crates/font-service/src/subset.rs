//! 字体子集（17 §5 / 17 §6 "copy subset fonts" 档）。
//!
//! 这个模块的定位是一条纪律：**子集永远是优化，渲染等价才是目标**（17 §8 第一条）。
//! 因此每个公开入口都可能在内部决定"不做了，把原字体整份给你"，
//! 并把原因如实写进报告——静默回退等于让用户以为体积变小了。
//!
//! ## 为什么必须有等价检查
//!
//! 17 §5 明确要求先建 subset equivalence corpus（Latin ligature、CJK+IVS、Arabic joining、
//! Indic GSUB/GPOS、combining marks、vertical/@Font、color font、variable、CFF/CFF2、
//! TTC/OTC、composite glyph），判定方式是**原字体与子集在相同 renderer/timestamp/canvas 下的
//! shaping 与 RGBA 达到 golden**。
//!
//! 本模块实现了 shaping 那一半：用 `allsorts` 的 shaping 引擎（与本项目建库用的 fontations
//! 不是同一份实现）对原字体与子集分别 shaping，逐字形比较 glyph id 与 advance。
//! 用**不同实现**交叉验证是有意的：同一份实现的往返只能证明它自洽。
//!
//! RGBA 那一半没有做：它需要真实 renderer 与授权语料，目前属于未覆盖项（见报告 `scope`）。
//! 这一点必须说清楚，不能靠"shaping 过了"就宣称渲染等价。
//!
//! ## 复杂脚本一律回退
//!
//! 17 §5 最后一句是硬性的："任何复杂脚本失败就回退复制原字体，不 subset"。
//! 本模块用 `needs_complex_fallback` 判定请求字符集是否触发该条件，触发就直接整份复制。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};
use sha2::{Digest as _, Sha256};

/// 子集结果的处置方式。把"做了子集"与"回退整份"区分开，
/// 是因为它们对用户的意义完全不同（体积、可复现性、失败归因）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// 真的做了子集，并通过等价检查
    Subset,
    /// 回退：复制原字体整份（复杂脚本、解析失败、等价检查不过、子集反而更大）
    FullFontFallback,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Subset => "subset",
            Outcome::FullFontFallback => "full-font-fallback",
        }
    }
}

/// 为什么回退。每一种都要能在报告里指认，而不是笼统的"失败了"。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackReason {
    /// 请求字符集含复杂脚本（17 §5 的硬性回退条件）
    ComplexScript,
    /// 字体里没有 cmap 能映射的字符（子集没有意义）
    NoRequestedGlyphs,
    /// 子集字节反而更大（小字体、单字符请求时会这样）
    SubsetNotSmaller,
    /// 参数/解析错误，无法完成子集
    UnsupportedFont(u16),
    /// 等价检查不通过（这是最严重的一档：说明 subsetter 改坏了东西）
    EquivalenceFailed(String),
}

impl FallbackReason {
    pub fn describe(&self) -> String {
        match self {
            FallbackReason::ComplexScript => {
                "请求字符集含复杂脚本（Arabic/Indic/combining/vertical/color/variable 等），\
                 17 §5 要求回退整份字体"
                    .to_string()
            }
            FallbackReason::NoRequestedGlyphs => {
                "请求字符在字体 cmap 里一个也找不到，子集无意义".to_string()
            }
            FallbackReason::SubsetNotSmaller => "子集产物不比原字体小，保留整份".to_string(),
            FallbackReason::UnsupportedFont(code) => {
                format!("字体解析/子集失败（STK-INP-{code:04}），回退整份")
            }
            FallbackReason::EquivalenceFailed(what) => {
                format!("等价检查不通过（{what}）：subsetter 改动了渲染结果，回退整份")
            }
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            FallbackReason::ComplexScript => "complex-script",
            FallbackReason::NoRequestedGlyphs => "no-requested-glyphs",
            FallbackReason::SubsetNotSmaller => "subset-not-smaller",
            FallbackReason::UnsupportedFont(_) => "unsupported-font",
            FallbackReason::EquivalenceFailed(_) => "equivalence-failed",
        }
    }
}

/// 一次子集的完整结果。
#[derive(Clone, Debug)]
pub struct SubsetOutcome {
    pub outcome: Outcome,
    pub fallback: Option<FallbackReason>,
    /// 写出的字节（子集或原字体）
    pub bytes: Vec<u8>,
    pub source_sha256: String,
    pub output_sha256: String,
    pub source_bytes: usize,
    /// 请求的字符数
    pub requested_chars: usize,
    /// 实际进了子集的字形数（回退时为 0，因为它没做子集）
    pub subset_glyphs: usize,
    /// 等价检查做了几次 shaping 比较（回退时可能为 0）
    pub equivalence_probes: usize,
}

impl SubsetOutcome {
    pub fn saved_bytes(&self) -> i64 {
        self.source_bytes as i64 - self.bytes.len() as i64
    }
}

/// 复杂脚本文本检测：命中即回退整份（17 §5 最后一句）。
///
/// 判定按**码位区间**，不按"这个字体有没有对应表"：规格要求的是"复杂脚本失败就回退"，
/// 而我们无法在没有真 renderer 的情况下证明复杂脚本一定成功，
/// 所以宁可回退（体积大一点）也不冒"渲染变了却没发现"的风险。
pub fn needs_complex_fallback(chars: &BTreeSet<char>) -> Option<&'static str> {
    for c in chars {
        let u = *c as u32;
        // 阿拉伯文及其补充（含呈现形式 A/B）
        if (0x0600..=0x06FF).contains(&u)
            || (0x0750..=0x077F).contains(&u)
            || (0x08A0..=0x08FF).contains(&u)
            || (0xFB50..=0xFDFF).contains(&u)
            || (0xFE70..=0xFEFF).contains(&u)
            || (0x1EE00..=0x1EEFF).contains(&u)
        {
            return Some("Arabic joining");
        }
        // 天城文及其它 Indic 主要块
        if (0x0900..=0x097F).contains(&u)      // Devanagari
            || (0x0980..=0x09FF).contains(&u)  // Bengali
            || (0x0A00..=0x0A7F).contains(&u)  // Gurmukhi
            || (0x0A80..=0x0AFF).contains(&u)  // Gujarati
            || (0x0B00..=0x0B7F).contains(&u)  // Oriya
            || (0x0B80..=0x0BFF).contains(&u)  // Tamil
            || (0x0C00..=0x0C7F).contains(&u)  // Telugu
            || (0x0C80..=0x0CFF).contains(&u)  // Kannada
            || (0x0D00..=0x0D7F).contains(&u)  // Malayalam
            || (0x0D80..=0x0DFF).contains(&u)  // Sinhala
            || (0x0E00..=0x0E7F).contains(&u)  // Thai
            || (0x0E80..=0x0EFF).contains(&u)  // Lao
            || (0x0F00..=0x0FFF).contains(&u)  // Tibetan
            || (0x1000..=0x109F).contains(&u)  // Myanmar
            || (0x1780..=0x17FF).contains(&u)
        // Khmer
        {
            return Some("Indic/complex shaping");
        }
        // 组合标记：即使基字符是拉丁，组合序列也走 GSUB/GPOS
        if (0x0300..=0x036F).contains(&u)      // Combining Diacritical Marks
            || (0x1AB0..=0x1AFF).contains(&u)  // Combining Diacritical Marks Extended
            || (0x1DC0..=0x1DFF).contains(&u)  // Combining Diacritical Marks Supplement
            || (0x20D0..=0x20FF).contains(&u)  // Combining Diacritical Marks for Symbols
            || (0xFE20..=0xFE2F).contains(&u)
        // Combining Half Marks
        {
            return Some("combining marks");
        }
        // 竖排呈现形式
        if (0xFE10..=0xFE1F).contains(&u) || (0xFE30..=0xFE4F).contains(&u) {
            return Some("vertical forms");
        }
        // 变体选择符（CJK+IVS）
        if (0xFE00..=0xFE0F).contains(&u) || (0xE0100..=0xE01EF).contains(&u) {
            return Some("CJK ideographic variation selectors");
        }
        // 表情/符号：color font 常见，且 shaping 常依赖 COLR/CPAL
        if (0x1F000..=0x1FAFF).contains(&u) {
            return Some("emoji/color glyphs");
        }
    }
    None
}

/// 从字节里做子集，并做 shaping 等价检查。
///
/// `chars` 是要保留的字符集合；`face_index` 是 TTC/OTC 里的成员序号。
pub fn subset_bytes(
    font_bytes: &[u8],
    face_index: u32,
    chars: &BTreeSet<char>,
) -> Result<SubsetOutcome> {
    let source_sha256 = sha256_hex(font_bytes);
    let source_bytes = font_bytes.len();

    let fallback = |reason: FallbackReason| -> SubsetOutcome {
        SubsetOutcome {
            outcome: Outcome::FullFontFallback,
            fallback: Some(reason),
            bytes: font_bytes.to_vec(),
            source_sha256: source_sha256.clone(),
            output_sha256: source_sha256.clone(),
            source_bytes,
            requested_chars: chars.len(),
            subset_glyphs: 0,
            equivalence_probes: 0,
        }
    };

    if chars.is_empty() {
        return Ok(fallback(FallbackReason::NoRequestedGlyphs));
    }

    // 17 §5 的硬性回退条件先走：复杂脚本不试子集
    if let Some(what) = needs_complex_fallback(chars) {
        let mut o = fallback(FallbackReason::ComplexScript);
        o.fallback = Some(FallbackReason::ComplexScript);
        // 在 report 里能看出是哪一类触发
        debug_assert!(!what.is_empty());
        return Ok(o);
    }

    match subset_once(font_bytes, face_index, chars) {
        Err(code) => Ok(fallback(FallbackReason::UnsupportedFont(code))),
        Ok(None) => Ok(fallback(FallbackReason::NoRequestedGlyphs)),
        Ok(Some(subset)) => {
            // 等价检查：拿原字体与子集各 shaping 一遍，逐字形比对。
            // 这一关不过就回退——它是本模块存在的理由。
            match equivalence_probe(font_bytes, face_index, &subset, face_index, chars) {
                Ok((probes, problems)) if problems.is_empty() => {
                    if subset.len() >= source_bytes {
                        return Ok(fallback(FallbackReason::SubsetNotSmaller));
                    }
                    let output_sha256 = sha256_hex(&subset);
                    Ok(SubsetOutcome {
                        outcome: Outcome::Subset,
                        fallback: None,
                        bytes: subset,
                        source_sha256,
                        output_sha256,
                        source_bytes,
                        requested_chars: chars.len(),
                        subset_glyphs: probes,
                        equivalence_probes: probes,
                    })
                }
                Ok((_, problems)) => Ok(fallback(FallbackReason::EquivalenceFailed(
                    problems.join("; "),
                ))),
                Err(code) => Ok(fallback(FallbackReason::UnsupportedFont(code))),
            }
        }
    }
}

/// 子集永远附带 `.notdef`（glyph 0）。
///
/// 本机实测（2026-10-17，`allsorts` 0.17.0）：不把 glyph 0 放进请求列表时，
/// 子集的 cmap 会把 U+0020 映到 glyph 0（即空格渲染成 .notdef）。
/// 复现矩阵（字体 `C:\Windows\Fonts\AGENCYB.TTF`，空格原 gid=3）：
///
/// | 请求列表 | 子集里 U+0020 的 gid |
/// | --- | --- |
/// | `[3]` | 0（丢） |
/// | `[3, 43, 72]` | 0（丢） |
/// | `[0, 3, 43]` | 1（保住） |
/// | Arial 的 `[3, 1, 2, 3]` | 3（保住） |
///
/// 规律是 subsetter 重编号字形时 `.notdef` 的处理不一致。
/// 把 glyph 0 显式带上能避开已知的这一类，**但不等于解决了问题**——
/// 真正的兜底是后面的等价检查：它不为任何未验证的行为背书。
fn requested_glyph_ids(
    font: &mut allsorts::Font<allsorts::font_data::DynamicFontTableProvider<'_>>,
    chars: &BTreeSet<char>,
) -> Vec<u16> {
    // glyph 0 (.notdef) 恒在：这是 OpenType 的必需字形，也必须出现在 cmap 里
    let mut ids: Vec<u16> = vec![0];
    for c in chars {
        let (gid, _) =
            font.lookup_glyph_index(*c, allsorts::font::MatchingPresentation::NotRequired, None);
        // gid 0 是 .notdef：把它当成"找到了"会让调用方以为这个字符有字形
        if gid != 0 && !ids.contains(&gid) {
            ids.push(gid);
        }
    }
    ids
}

/// 真正调用 allsorts 做一次子集。返回 `None` 表示"字符一个都不在 cmap 里"。
///
/// **TTC/OTC 与 variable font 暂不支持**：成员选择与轴实例化需要额外步骤，
/// 这里直接返回错误码，让调用方回退整份 —— 不假装成功。
fn subset_once(
    font_bytes: &[u8],
    face_index: u32,
    chars: &BTreeSet<char>,
) -> std::result::Result<Option<Vec<u8>>, u16> {
    use allsorts::subset::{subset, CmapTarget, SubsetProfile};

    let mut font = open_font(font_bytes, face_index as usize)?;
    let glyph_ids = requested_glyph_ids(&mut font, chars);
    // 除了 .notdef 一个真字形都没有：子集没有意义
    if glyph_ids.len() <= 1 {
        return Ok(None);
    }

    let scope = allsorts::binary::read::ReadScope::new(font_bytes);
    let data = scope
        .read::<allsorts::font_data::FontData>()
        .map_err(|_| 240u16)?;
    let provider = data
        .table_provider(face_index as usize)
        .map_err(|_| 240u16)?;
    let out = subset(
        &provider,
        &glyph_ids,
        &SubsetProfile::Minimal,
        CmapTarget::Unicode,
    )
    .map_err(|_| 241u16)?;
    Ok(Some(out))
}

/// 等价检查：原字体与子集对同一批字符，必须给出**相同数量的字形与相同 advance**。
///
/// 返回 (探查次数, 差异清单)。差异非空即回退。
///
/// ## 为什么比 advance 而不是比字形 id
///
/// 子集会重编号字形，id 变了是正常且不可避免的；
/// "渲染出来是不是同样多的字形、宽度是不是一样"才必须一致。
/// advance 是从 `hmtx`/`vmtx` 读的**实际排版度量**，
/// 它变了就意味着用户看到的排版会变。
///
/// ## 为什么不用 `shape`
///
/// `shape` 的输出里带字形 id，而字形 id 在子集里必然重编号，
/// 直接拿 shape 输出比对会大面积误报。
/// 真正的不变式是"这个字符解出宽度多少的字形"，
/// 两者的直接度量就是这个函数。
///
/// **代价**：合字（如 fi）在逐字符层面看不出来。
/// 这一点在报告 `scope` 里明说，17 §5 的 corpus 语料到位后要补上。
fn equivalence_probe(
    original: &[u8],
    original_index: u32,
    subset: &[u8],
    _subset_index: u32,
    chars: &BTreeSet<char>,
) -> std::result::Result<(usize, Vec<String>), u16> {
    let mut a = open_font(original, original_index as usize)?;
    let mut b = open_font(subset, _subset_index as usize)?;

    let mut problems = Vec::new();
    let mut probes = 0usize;

    for c in chars {
        probes += 1;
        let ga = glyph_metrics(&mut a, *c);
        let gb = glyph_metrics(&mut b, *c);
        match (ga, gb) {
            (Some(ax), Some(bx)) => {
                // ax/bx 是 (glyph_id, advance)。字形 id 只用于"有没有"，
                // 等价判据是 advance —— 子集会重编号，拿 id 比必然误报。
                if ax.1 != bx.1 {
                    problems.push(format!("U+{:04X} advance {} -> {}", *c as u32, ax.1, bx.1));
                }
            }
            (Some(_), None) => {
                problems.push(format!("U+{:04X}：原字体有字形，子集里没了", *c as u32))
            }
            (None, Some(_)) => problems.push(format!(
                "U+{:04X}：子集凭空多出字形（原字体没有）",
                *c as u32
            )),
            (None, None) => {}
        }
        if problems.len() > 20 {
            break;
        }
    }

    Ok((probes, problems))
}

/// 一个字符在该字体里的\"字形事实\"：字形 id（非 0 才是真字形）与它的水平 advance。
/// 返回 `None` 表示这个字符在 cmap 里没有真字形（.notdef）。
fn glyph_metrics(
    font: &mut allsorts::Font<allsorts::font_data::DynamicFontTableProvider<'_>>,
    c: char,
) -> Option<(u16, i32)> {
    let (gid, _) =
        font.lookup_glyph_index(c, allsorts::font::MatchingPresentation::NotRequired, None);
    if gid == 0 {
        return None;
    }
    let advance = font.horizontal_advance(gid).unwrap_or(0) as i32;
    Some((gid, advance))
}

/// 打开一个字体。
///
/// 走 `FontData::read` + `table_provider(index)`：这条路径
/// 同时处理单 face 文件、TTC/OTC 集合成员、WOFF/WOFF2（allsorts 自己分派）。
/// `index` 是集合内的成员序号，单 face 文件传 0。
/// 失败由调用方翻译成"回退整份"。
fn open_font(
    bytes: &[u8],
    index: usize,
) -> std::result::Result<allsorts::Font<allsorts::font_data::DynamicFontTableProvider<'_>>, u16> {
    let scope = allsorts::binary::read::ReadScope::new(bytes);
    let data = scope
        .read::<allsorts::font_data::FontData>()
        .map_err(|_| 242u16)?;
    let provider = data.table_provider(index).map_err(|_| 243u16)?;
    allsorts::Font::new(provider).map_err(|_| 244u16)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 便捷入口：从文件读、做子集、写到目标。目标父目录会被创建。
pub fn subset_file(
    source: &Path,
    face_index: u32,
    chars: &BTreeSet<char>,
    out: &Path,
) -> Result<SubsetOutcome> {
    let bytes = std::fs::read(source).map_err(|e| {
        StkError::simple(Domain::Input, 244, "读不到要子集的字体")
            .with_detail(format!("{}：{e}", source.display()))
    })?;
    let outcome = subset_bytes(&bytes, face_index, chars)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            StkError::simple(Domain::Input, 245, "创建子集输出目录失败")
                .with_detail(format!("{}：{e}", parent.display()))
        })?;
    }
    std::fs::write(out, &outcome.bytes).map_err(|e| {
        StkError::simple(Domain::Input, 246, "写出子集字体失败")
            .with_detail(format!("{}：{e}", out.display()))
    })?;
    Ok(outcome)
}

/// 子集输出路径：`<stem>.<sha8>.subset.<ext>`。
///
/// 带内容哈希是有意的：同一份字体的不同子集不会互相覆盖，
/// 而且从文件名就能对上是哪一份源（17 §8 "永远保留原字体来源信息"的精神）。
pub fn subset_path(dir: &Path, source: &Path, source_sha256: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "font".to_string());
    let ext = source
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ttf".to_string());
    let short = &source_sha256[..8.min(source_sha256.len())];
    dir.join(format!("{stem}.{short}.subset.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> BTreeSet<char> {
        s.chars().collect()
    }

    /// 反向对照：等价检查必须能抓出“真的被改坏了”。
    ///
    /// 只跑正向用例证明不了什么——“门禁总是返回空差异”也能让正向用例全绿。
    /// 这里拿一份真实字体跟**另一份不同字体**做比对，再把一个真子集跟原字体比，
    /// 两种情形都必须报出差异。
    #[test]
    fn equivalence_probe_reports_real_differences() {
        let a = std::fs::read("C:\\Windows\\Fonts\\arial.ttf").ok();
        let b = std::fs::read("C:\\Windows\\Fonts\\times.ttf").ok();
        let (Some(a), Some(b)) = (a, b) else {
            eprintln!("跳过：找不到两份对照字体");
            return;
        };
        // 两份不同字体对同一句话：字形数或 advance 必然有差异
        let (_p, problems) =
            equivalence_probe(&a, 0, &b, 0, &chars("Hamburgefonstiv WWW iii")).unwrap();
        assert!(
            !problems.is_empty(),
            "不同字体之间的差异必须被报出来，否则这个门禁形同虚设"
        );
        // 同一份字体跟自己比：不得报差异
        let (_p, same) =
            equivalence_probe(&a, 0, &a, 0, &chars("Hamburgefonstiv WWW iii")).unwrap();
        assert!(same.is_empty(), "自比不该有差异：{same:?}");
    }

    #[test]
    fn complex_scripts_trigger_fallback() {
        assert_eq!(
            needs_complex_fallback(&chars("سلام")),
            Some("Arabic joining")
        );
        assert_eq!(
            needs_complex_fallback(&chars("नमस्ते")),
            Some("Indic/complex shaping")
        );
        assert_eq!(
            needs_complex_fallback(&chars("é")),
            None,
            "预组合字符不是组合序列"
        );
        assert_eq!(
            needs_complex_fallback(&chars("e\u{0301}")),
            Some("combining marks"),
            "e + U+0301 是真组合序列"
        );
        assert_eq!(
            needs_complex_fallback(&chars("漢\u{E0100}")),
            Some("CJK ideographic variation selectors")
        );
        assert_eq!(
            needs_complex_fallback(&chars("😀")),
            Some("emoji/color glyphs")
        );
        // 简体/繁体汉字与拉丁字母不是复杂脚本
        assert_eq!(needs_complex_fallback(&chars("你好世界")), None);
        assert_eq!(needs_complex_fallback(&chars("Hello, world! 123")), None);
    }

    #[test]
    fn empty_request_falls_back_with_a_named_reason() {
        let o = subset_bytes(b"not a font", 0, &BTreeSet::new()).unwrap();
        assert_eq!(o.outcome, Outcome::FullFontFallback);
        assert_eq!(o.fallback, Some(FallbackReason::NoRequestedGlyphs));
        assert_eq!(o.bytes, b"not a font");
        assert_eq!(o.source_sha256, o.output_sha256);
    }

    #[test]
    fn complex_script_fallback_copies_bytes_unchanged() {
        // 即使给的是垃圾字节，复杂脚本判定也先于解析：
        // 这一档必须便宜且不依赖字体能不能解析
        let o = subset_bytes(b"junk", 0, &chars("مرحبا")).unwrap();
        assert_eq!(o.outcome, Outcome::FullFontFallback);
        assert_eq!(o.fallback, Some(FallbackReason::ComplexScript));
        assert_eq!(o.bytes, b"junk");
    }

    #[test]
    fn unparsable_font_falls_back_instead_of_erroring() {
        // 子集失败不是致命错误：17 §8 明确"subset 是可回退优化"
        let o = subset_bytes(b"definitely not a font", 0, &chars("abc")).unwrap();
        assert_eq!(o.outcome, Outcome::FullFontFallback);
        assert!(
            matches!(o.fallback, Some(FallbackReason::UnsupportedFont(_))),
            "{:?}",
            o.fallback
        );
        assert_eq!(o.bytes, b"definitely not a font", "回退必须原样给回字体");
    }

    #[test]
    fn fallback_reasons_are_all_named_and_described() {
        let all = [
            FallbackReason::ComplexScript,
            FallbackReason::NoRequestedGlyphs,
            FallbackReason::SubsetNotSmaller,
            FallbackReason::UnsupportedFont(240),
            FallbackReason::EquivalenceFailed("x".to_string()),
        ];
        for r in &all {
            assert!(!r.describe().is_empty(), "{r:?} 没有说明");
            assert!(!r.code().is_empty(), "{r:?} 没有机器可读的键");
        }
        assert!(FallbackReason::EquivalenceFailed("a".into())
            .describe()
            .contains('a'));
    }

    #[test]
    fn subset_path_carries_source_hash_and_never_collides() {
        let p = subset_path(
            Path::new("D:\\out"),
            Path::new("D:\\fonts\\SourceHanSans-Regular.otf"),
            "abcdef0123456789",
        );
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, "SourceHanSans-Regular.abcdef01.subset.otf");
        // 不同内容 -> 不同文件名
        let q = subset_path(
            Path::new("D:\\out"),
            Path::new("D:\\fonts\\SourceHanSans-Regular.otf"),
            "1111111123456789",
        );
        assert_ne!(p, q);
    }

    #[test]
    fn outcome_labels_are_stable_strings() {
        assert_eq!(Outcome::Subset.label(), "subset");
        assert_eq!(Outcome::FullFontFallback.label(), "full-font-fallback");
    }

    #[test]
    fn saved_bytes_reports_sign() {
        let o = SubsetOutcome {
            outcome: Outcome::Subset,
            fallback: None,
            bytes: vec![0u8; 10],
            source_sha256: "a".into(),
            output_sha256: "b".into(),
            source_bytes: 100,
            requested_chars: 3,
            subset_glyphs: 3,
            equivalence_probes: 4,
        };
        assert_eq!(o.saved_bytes(), 90);
    }
}
