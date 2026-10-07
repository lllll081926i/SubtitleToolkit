//! 匹配评级（17 §4）：把"字幕要这个 face"变成"这台机器会给什么"。
//!
//! 评分次序按规格来，不是按方便来：家族/face 名精确 → typographic 与本地化别名 →
//! 字重距离 → 斜体不符的罚分 → 宽度（stretch）→ 覆盖。
//!
//! 第 7 项"renderer 实际解析结果"这一层**没有**用到：我们不去问 libass 的字体选择器
//! （那需要 DirectWrite/fontconfig 视角），所以 `Missing` 的含义是"按名字在这台机器的
//! 字体库里找不到"，不等于"渲染一定失败"（libass 还会走自己的 fallback）。
//! 这个边界写进报告的 `why` 与 `scope` 里，不藏在文档外。
//!
//! 六档里的 `MissingGlyph` 是改判出来的：名字对上（Exact/Alias）或走近似替代（Substituted）
//! 之后，如果赢的那条 face 的 cmap 覆盖不了正文，verdict 就变成 `MissingGlyph`，
//! 原本的匹配方式保留在 `face.match_by`。理由是"说名字对上了"与"说画不出字"不能各说一半。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use core_model::error::Result;
use serde::Serialize;

use crate::db::{CacheMode, CacheState, FaceRecord, FontDb, NameMatch, FONT_CHANGED_AFTER_SCAN};
use crate::usage::FaceRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 家族名直接对上
    Exact,
    /// 靠 PostScript 名/全名/本地化名对上（17 §2 说 localized family 必须能匹配，就是这条）
    Alias,
    /// 没找到同名 face，给了一个"名字最接近 + 字重最近"的替代，并说明差异
    Substituted,
    /// 字体库里根本没有相关的 face
    Missing,
    /// 同一个名字对上多个不同家族的 face，必须由用户决定
    Ambiguous,
    /// 名字对上了，但对上的 face 画不出正文里的某些字符（17 §4 的第 5 档）
    MissingGlyph,
}

impl Verdict {
    /// 17 §4 的六档，顺序照文档。对外报告与测试都从这里取，不再各抄一份：
    /// 手抄的清单会在实现变化后继续宣称旧事实（ISSUES AC4 就是这么发生的）。
    pub const SPEC_STATES: [Verdict; 6] = [
        Verdict::Exact,
        Verdict::Alias,
        Verdict::Substituted,
        Verdict::Missing,
        Verdict::Ambiguous,
        Verdict::MissingGlyph,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FaceSummary {
    pub family: String,
    pub style_name: String,
    pub weight: u16,
    pub italic: bool,
    pub width: u16,
    /// 展示用的标签（集合成员带 `#index`）
    pub file: String,
    /// 真实文件路径：交付层要按它复制，不能拿标签去猜
    pub path: String,
    pub face_index: u32,
    pub collection_len: u32,
    /// 靠哪一种名字对上的：family / win32 / postscript / full / localized
    pub match_by: String,
    pub variable: bool,
    /// 扫下来那份文件的内容哈希。交付与缓存身份都靠它验"还是同一份字节"
    pub source_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontMatch {
    pub request: String,
    pub bold: bool,
    pub italic: bool,
    pub chars: String,
    pub pen: usize,
    pub sources: Vec<String>,
    pub verdict: Verdict,
    pub face: Option<FaceSummary>,
    pub candidates: Vec<FaceSummary>,
    pub missing_glyphs: String,
    pub why: String,
}

impl FaceSummary {
    fn of(rec: &FaceRecord, how: NameMatch) -> FaceSummary {
        FaceSummary {
            family: rec.family.clone(),
            style_name: rec.style_name.clone(),
            weight: rec.weight,
            italic: rec.italic,
            width: rec.width,
            file: rec.label(),
            path: rec.file.display().to_string(),
            face_index: rec.face_index,
            collection_len: rec.collection_len,
            match_by: match how {
                NameMatch::Family => "family".to_string(),
                NameMatch::Win32Family => "win32".to_string(),
                NameMatch::PostScript => "postscript".to_string(),
                NameMatch::FullName => "full".to_string(),
                NameMatch::Localized => "localized".to_string(),
                NameMatch::No => "none".to_string(),
            }
            .to_string(),
            variable: rec.variable,
            source_sha256: rec.sha256.clone(),
        }
    }
}

/// 一次评级的排序键：越小越好。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    tier: u8,
    weight_gap: i32,
    italic_penalty: u8,
    width_penalty: u16,
}

fn tier_of(m: NameMatch) -> u8 {
    match m {
        NameMatch::Family => 0,
        NameMatch::Win32Family => 1,
        NameMatch::PostScript => 2,
        NameMatch::FullName => 3,
        NameMatch::Localized => 4,
        NameMatch::No => 9,
    }
}

fn score(rec: &FaceRecord, how: NameMatch, want_weight: u16, want_italic: bool) -> Score {
    Score {
        tier: tier_of(how),
        weight_gap: (rec.weight as i32) - (want_weight as i32),
        italic_penalty: if rec.italic != want_italic { 1 } else { 0 },
        // 宽度是 OS/2.usWidthClass 的 1…9 等级，**5 才是常规**。
        // 这里以前按 100 计罚分（跟着那条"100…200"的错误注释写的），结果是等级越大
        // （越宽）越"接近常规"，方向都反了。真实字体几乎全是 5，所以没人发现。
        width_penalty: (rec.width as i32 - NORMAL_WIDTH_CLASS).unsigned_abs() as u16,
    }
}

/// `OS/2.usWidthClass` 的常规值（1…9 里的 5）。
const NORMAL_WIDTH_CLASS: i32 = 5;

fn abs_gap(s: &Score) -> i32 {
    s.weight_gap.abs()
}

/// `why` 是给人读的一句话，缺字清单可能有上千个字符：截断并指回完整字段。
/// 截完必须比原文短，否则这段"给人类的说明"反而把关键行挤掉了。
fn shorten(s: &str) -> String {
    const KEEP: usize = 8;
    let n = s.chars().count();
    if n <= KEEP {
        return s.to_string();
    }
    let head: String = s.chars().take(KEEP).collect();
    format!("{head}…等 {n} 个（完整清单在 missing_glyphs）")
}

pub fn evaluate(db: &FontDb, requests: &[FaceRequest]) -> Result<Vec<FontMatch>> {
    let mut out = Vec::new();
    for req in requests {
        out.push(evaluate_one(db, req)?);
    }
    Ok(out)
}

/// 用现成的库解析请求，并且**缓存撒谎时自己重来**，不把内部一致性问题丢给用户去猜。
///
/// 唯一的发现地点就是使用点：`FontDb::missing_glyphs` 判覆盖时要重读那条 face 的文件，
/// 内容与记录不符会报 [`FONT_CHANGED_AFTER_SCAN`]（指纹只记大小与 mtime，同长度的原地覆盖
/// 它看不出来）。只有 `db` 本身是缓存来的（`cache == CacheState::Hit`）才值得重来 ——
/// 现扫的库就是刚才读的字节，再报不一致说明文件正在被并发覆盖，那是照实报错的事，
/// 不是重试能解决的事。重来那次换的是 `CacheMode::Refresh`：读当前字节重建，并把新索引写回缓存。
///
/// 代价说清楚：命中且没撒谎时省掉整座库的解析；撒谎时多付一次全量扫描，仍然只付一次。
pub fn evaluate_or_rescan(
    db: &mut FontDb,
    dirs: &[&Path],
    registered: &[PathBuf],
    requests: &[FaceRequest],
) -> Result<Vec<FontMatch>> {
    let first = evaluate(db, requests);
    if let Err(e) = &first {
        if e.code == FONT_CHANGED_AFTER_SCAN && db.cache.is_hit() {
            let mut fresh = FontDb::scan_cached(dirs, registered, CacheMode::Refresh);
            fresh.cache = CacheState::Refreshed;
            let matches = evaluate(&fresh, requests)?;
            *db = fresh;
            return Ok(matches);
        }
    }
    first
}

fn evaluate_one(db: &FontDb, req: &FaceRequest) -> Result<FontMatch> {
    let want_weight = if req.bold { 700 } else { 400 };
    let mut named: Vec<(Score, usize, NameMatch)> = Vec::new();
    for (idx, rec) in db.faces.iter().enumerate() {
        let how = rec.matches_name(&req.family);
        if how != NameMatch::No {
            named.push((score(rec, how, want_weight, req.italic), idx, how));
        }
    }
    named.sort_by_key(|(s, idx, _)| (s.tier, abs_gap(s), s.italic_penalty, s.width_penalty, *idx));

    let mut missing_glyphs = BTreeSet::new();
    let mut finish = |mut verdict: Verdict,
                      face: Option<(usize, NameMatch)>,
                      candidates: Vec<(usize, NameMatch)>,
                      mut why: String|
     -> Result<FontMatch> {
        if let Some((idx, _)) = face {
            for c in db.missing_glyphs(idx, &req.chars)? {
                missing_glyphs.insert(c);
            }
        }
        // 17 §4 把 "Missing Glyph" 也列成一档。它的优先级按"哪一个结论会误导人"来定：
        // 名字本来就对上了却说 Exact，用户会以为万事大吉 —— 所以 Exact/Alias/Substituted
        // 遇到缺字一律改判 MissingGlyph，并把原本的名字匹配方式留在 face.match_by 里。
        // Ambiguous 不改：还没确定用哪条 face，谈不上"这条 face 缺字"。
        if !missing_glyphs.is_empty() && verdict != Verdict::Ambiguous {
            why.push_str(&format!(
                "；但本 face 的 cmap 覆盖不了 {} 个字符，渲染时会走 fallback 或豆腐块",
                missing_glyphs.len()
            ));
            why.push_str(&format!(
                "（缺：{}）",
                shorten(&missing_glyphs.iter().collect::<String>())
            ));
            verdict = Verdict::MissingGlyph;
        }
        Ok(FontMatch {
            request: req.family.clone(),
            bold: req.bold,
            italic: req.italic,
            chars: req.chars.iter().collect(),
            pen: req.pen,
            sources: req.sources.clone(),
            verdict,
            face: face.map(|(i, how)| FaceSummary::of(&db.faces[i], how)),
            candidates: candidates
                .into_iter()
                .map(|(i, how)| FaceSummary::of(&db.faces[i], how))
                .collect(),
            missing_glyphs: missing_glyphs.iter().collect(),
            why,
        })
    };

    if !named.is_empty() {
        let best = named[0].0.tier;
        let top: Vec<(usize, NameMatch)> = named
            .iter()
            .filter(|(s, _, _)| s.tier == best)
            .map(|(_, i, how)| (*i, *how))
            .collect();
        let distinct_families: BTreeSet<String> = top
            .iter()
            .map(|(i, _)| db.faces[*i].family.to_ascii_lowercase())
            .collect();
        let (win_idx, win_how) = top[0];
        let rec = &db.faces[win_idx];
        let verdict = if best >= 2 {
            Verdict::Alias
        } else if distinct_families.len() > 1 {
            Verdict::Ambiguous
        } else {
            Verdict::Exact
        };
        let mut why = format!(
            "名字对上 {} 条 face（最佳：{}，字重 {}，斜体 {}）",
            named.len(),
            rec.label(),
            rec.weight,
            rec.italic
        );
        if verdict == Verdict::Ambiguous {
            why.push_str("；同名指向不同家族，必须由用户选");
        }
        if rec.weight != want_weight {
            why.push_str(&format!("；请求字重 {want_weight}，本 face {}", rec.weight));
        }
        if rec.italic != req.italic {
            why.push_str("；斜体不符（合成斜体的风险由渲染器决定）");
        }
        return finish(verdict, Some((win_idx, win_how)), top, why);
    }

    // 没有同名 face：先给"家族名有共同词"的近似候选，再退回 Missing。
    let want_tokens: BTreeSet<String> = tokens(&req.family);
    let mut near: Vec<(usize, u8)> = Vec::new();
    for (idx, rec) in db.faces.iter().enumerate() {
        let have = tokens(&rec.family);
        let shared = want_tokens.iter().filter(|t| have.contains(*t)).count();
        if shared > 0 {
            near.push((idx, shared as u8));
        }
    }
    near.sort_by(|a, b| {
        let ra = &db.faces[a.0];
        let rb = &db.faces[b.0];
        (b.1, (ra.weight as i32 - want_weight as i32).abs())
            .cmp(&(a.1, (rb.weight as i32 - want_weight as i32).abs()))
            .then_with(|| ra.family.cmp(&rb.family))
    });
    if let Some((idx, shared)) = near.first() {
        let rec = &db.faces[*idx];
        return finish(
            Verdict::Substituted,
            Some((*idx, NameMatch::No)),
            near.iter()
                .take(4)
                .map(|(i, _)| (*i, NameMatch::No))
                .collect(),
            format!(
                "这台机器上没有名为 {} 的 face，按共同词（{shared}）与字重距离选了 {}；\
                 这是**替代**不是等价，渲染结果会不同（17 §10：不得静默换 renderer 后声称等价）",
                req.family,
                rec.label()
            ),
        );
    }
    finish(
        Verdict::Missing,
        None,
        Vec::new(),
        format!(
            "字体库里找不到与 {} 相关的 face（扫过 {} 个目录、{} 条 face）。\
             只说明按名字找不到；libass 自己的 fallback 不在此判断范围内",
            req.family,
            db.roots.len(),
            db.faces.len()
        ),
    )
}

fn tokens(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1)
        .map(|t| t.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_order_follows_the_spec_not_convenience() {
        assert!(tier_of(NameMatch::Family) < tier_of(NameMatch::Win32Family));
        assert!(tier_of(NameMatch::Win32Family) < tier_of(NameMatch::PostScript));
        assert!(tier_of(NameMatch::Localized) < tier_of(NameMatch::No));
        let s = Score {
            tier: 0,
            weight_gap: -300,
            italic_penalty: 1,
            width_penalty: 0,
        };
        // 名字层级永远优先于字重距离：先找回家族，再谈粗细
        assert_eq!(s.tier, 0);
        assert_eq!(abs_gap(&s), 300);
    }

    #[test]
    fn tokens_split_on_punctuation_and_drop_single_letters() {
        let t = tokens("Arial Black");
        assert!(t.contains("arial") && t.contains("black"));
        assert!(!t.contains("a"));
        assert_eq!(tokens("Segoe UI"), {
            let mut x = BTreeSet::new();
            x.insert("segoe".to_string());
            x.insert("ui".to_string());
            x
        });
    }

    /// 17 §4 的六档就是对外契约：JSON 里的拼法必须一次到位，
    /// 下游（CLI 计数、GUI 上色、脚本判断）都按这六个字符串写。
    #[test]
    fn the_six_spec_states_have_stable_serialization_names() {
        let all = Verdict::SPEC_STATES;
        let got: Vec<String> = all
            .iter()
            .map(|v| {
                serde_json::to_value(v)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            got,
            vec![
                "exact",
                "alias",
                "substituted",
                "missing",
                "ambiguous",
                "missing_glyph"
            ]
        );
    }

    /// 宽度是 1…9 的等级，5 是常规。这条测试锁的是"罚分基准"：
    /// 曾经的写法拿 100 当基准，于是 9（扩张）比 5（常规）更"接近常规"。
    #[test]
    fn width_penalty_is_measured_against_the_normal_stretch_class() {
        let mut rec = FaceRecord {
            file: std::path::PathBuf::from("x.ttf"),
            face_index: 0,
            collection_len: 1,
            family: "X".to_string(),
            win32_family: "X".to_string(),
            full_name: String::new(),
            postscript_name: String::new(),
            style_name: String::new(),
            family_aliases: Vec::new(),
            weight: 400,
            italic: false,
            width: 5,
            variable: false,
            axes: Vec::new(),
            num_glyphs: 10,
            units_per_em: 1000,
            sha256: String::new(),
            mtime_secs: None,
        };
        let normal = score(&rec, NameMatch::Family, 400, false);
        rec.width = 9;
        let expanded = score(&rec, NameMatch::Family, 400, false);
        rec.width = 1;
        let condensed = score(&rec, NameMatch::Family, 400, false);
        assert!(
            normal.width_penalty < expanded.width_penalty,
            "{normal:?} {expanded:?}"
        );
        assert!(
            normal.width_penalty < condensed.width_penalty,
            "{normal:?} {condensed:?}"
        );
        assert_eq!(normal.width_penalty, 0);
    }

    #[test]
    fn why_truncates_a_long_missing_list_but_points_at_the_full_one() {
        assert_eq!(shorten("你好"), "你好");
        let long = "字".repeat(40);
        let s = shorten(&long);
        // 截断的目的：给人读的那一行不能比原始清单还长
        assert!(s.chars().count() < long.chars().count(), "{s}");
        assert_eq!(
            s.chars().filter(|c| *c == '字').count(),
            8,
            "只留前 8 个：{s}"
        );
        assert!(
            s.contains("等 40 个") && s.contains("missing_glyphs"),
            "数量与完整字段的指路都要在：{s}"
        );
    }
}
