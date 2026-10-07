//! 字体环境身份（17 §7 最后一行：`render cache key` 必须包含**实际用到的字体字节哈希** +
//! resolver policy + renderer 版本）。
//!
//! 分工要说清楚，别把三件事混成一件：
//! * **字体字节哈希**：由这里给出 —— 扫描结果里每条被选中的 face 的 SHA-256；
//! * **resolver policy**：也由这里给出（扫了哪些根目录、读没读注册表、版本标记）；
//! * **renderer 版本**：由调用方给（provider 二进制的 SHA-256 已经在 `render_cache_key` 里）。
//!
//! 为什么这件事不是"锦上添花"：在补上它之前，同一个输入在同一台机器上换了字体版本
//! （升级、卸载、`--fonts-dir` 指到别处）渲染结果会变，但缓存 key 不变 ——
//! 那就是拿上一次的像素当这一次的交付物。AT-FONT-002 只做到了"判定覆盖时拒绝旧结论"，
//! 这一条补的是"复用时的身份"。
//!
//! 未解析到的请求也参与摘要：今天 "Missing" 的那家字体，明天被装上，渲染结果必然改变，
//! 所以缺席本身也是环境的一部分。

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::attachments::Attachment;
use crate::db::{CacheState, FontDb};
use crate::report::{FontMatch, Verdict};

/// 这份身份算法的版本。规则变了就要换值，让旧缓存自然失效而不是悄悄复用。
pub const POLICY_VERSION: &str = "stk-fontenv-v1";

#[derive(Clone, Debug, Serialize)]
pub struct FontEnv {
    /// 算法版本 + 扫描根 + 是否读注册表
    pub policy: String,
    /// 参与摘要的每一格（排序后）：命中的 face 或明确缺席的请求
    pub entries: Vec<String>,
    /// 上面所有内容的 SHA-256；这就是进 cache key 的那一维
    pub digest: String,
    pub faces_used: usize,
    pub faces_missing: usize,
    /// 脚本自己带的 `[Fonts]` 内嵌字体条数（17 §7 的第二句）。它们同样是"实际喂给渲染器的
    /// 字节"，所以每一份都按 名字+内容哈希 进摘要：只带内嵌字体的两份脚本不能共用同一份像素。
    pub attachments: usize,
    /// 这次的库是怎么来的（命中缓存 / 全量重扫 / 明确不用缓存）。
    /// 它是**成本事实**不是环境事实：不进摘要，只进报告，否则有没有缓存会让同一环境产生两个 key。
    pub cache: CacheState,
}

/// 由"扫描出来的库"与"这份脚本解析到的匹配结果"算出环境身份。
///
/// 只取**实际用到**的那些 face（17 §7 说的就是"实际"）：库里装了四百家字体而脚本只用三家，
/// 装第四家不应该让缓存失效。
pub fn identify(db: &FontDb, matches: &[FontMatch], attachments: &[Attachment]) -> FontEnv {
    let mut entries: Vec<String> = Vec::new();
    let mut faces_used = 0usize;
    let mut faces_missing = 0usize;
    for m in matches {
        match (&m.face, m.verdict == Verdict::Missing) {
            (Some(face), false) => {
                faces_used += 1;
                // 路径 + face 索引 + 内容哈希：TTC 里换一个成员、或文件被升级，都必须换 key
                entries.push(format!(
                    "hit|{}|{}|{}|{}|{}",
                    norm(&m.request),
                    m.bold as u8,
                    m.italic as u8,
                    face.path,
                    face.face_index
                ));
                entries.push(format!("sha|{}|{}", face.path, face.source_sha256));
            }
            _ => {
                // Alias/Substituted 也可能 face 为 None？不会——但"没 face 就是没解析到"，
                // 一律按缺席计入，宁可多算一条也不漏
                faces_missing += 1;
                entries.push(format!(
                    "miss|{}|{}|{}",
                    norm(&m.request),
                    m.bold as u8,
                    m.italic as u8
                ));
            }
        }
    }
    for a in attachments {
        // 内部名优先（libass 就是按它匹配的），读不到才用脚本里写的文件名
        entries.push(format!("att|{}|{}|{}", a.label(), a.sha256, a.len()));
    }
    entries.extend(db.roots.iter().map(|r| format!("root|{r}")));
    entries.sort();
    // policy 说的是"用哪套规则算的"，不是"扫了哪些来源"：来源在 entries 里，也进了摘要。
    // 混在一起会让同一套规则在不同机器上给出不同 policy 字符串，读的人反而分不清哪个才是关键。
    let policy = POLICY_VERSION.to_string();
    let mut h = Sha256::new();
    h.update(policy.as_bytes());
    for e in &entries {
        h.update(e.as_bytes());
        h.update([0u8]);
    }
    let digest = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    FontEnv {
        policy,
        entries,
        digest,
        faces_used,
        faces_missing,
        attachments: attachments.len(),
        cache: db.cache,
    }
}

/// 家族名里的空白会影响比对吗？不影响匹配（`matches_name` 自己会规范化），但会影响摘要。
/// 这里只把连续空白收成单个空格，让"同一个请求"在两份等价脚本里给出同一格。
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{FaceSummary, Verdict};
    use crate::usage::FaceRequest;

    fn attachment(name: &str, sha: &str) -> Attachment {
        Attachment {
            declared_name: name.to_string(),
            family: Some(name.to_string()),
            sha256: sha.to_string(),
            data: vec![0u8; 1234],
            parseable: true,
        }
    }

    /// 内嵌字体是脚本自带的输入：只带 `[Fonts]` 的两份脚本（内容不同）必须落到不同的
    /// 渲染身份上，否则缓存会把一份的像素发给另一份。
    #[test]
    fn an_embedded_font_changes_the_identity() {
        let db = db_with(&["R"]);
        let bare = identify(&db, &[hit("Arial", "a.ttf", "1")], &[]);
        let one = identify(
            &db,
            &[hit("Arial", "a.ttf", "1")],
            &[attachment("Eve.ttf", "a")],
        );
        let other = identify(
            &db,
            &[hit("Arial", "a.ttf", "1")],
            &[attachment("Eve.ttf", "b")],
        );
        let swapped = identify(
            &db,
            &[hit("Arial", "a.ttf", "1")],
            &[attachment("Adam.ttf", "a")],
        );
        assert_ne!(bare.digest, one.digest, "多一份内嵌字体就是另一个环境");
        assert_ne!(one.digest, other.digest, "内嵌字体内容变了必须换身份");
        assert_ne!(one.digest, swapped.digest, "名字也算身份的一部分");
        assert_eq!(one.attachments, 1);
        assert_eq!(bare.attachments, 0);
        // 顺序无关：摘要不能因为枚举次序而漂
        let both = identify(
            &db,
            &[hit("Arial", "a.ttf", "1")],
            &[attachment("Adam.ttf", "a"), attachment("Eve.ttf", "b")],
        );
        let flipped = identify(
            &db,
            &[hit("Arial", "a.ttf", "1")],
            &[attachment("Eve.ttf", "b"), attachment("Adam.ttf", "a")],
        );
        assert_eq!(both.digest, flipped.digest);
    }

    fn db_with(roots: &[&str]) -> FontDb {
        FontDb {
            faces: vec![],
            notes: vec![],
            roots: roots.iter().map(|s| s.to_string()).collect(),
            files_read: 0,
            registry_only: 0,
            cache: CacheState::Scanned,
        }
    }

    fn req(family: &str) -> FaceRequest {
        FaceRequest {
            family: family.to_string(),
            bold: false,
            italic: false,
            chars: ['你'].into_iter().collect(),
            pen: 1,
            sources: vec!["style:D".to_string()],
        }
    }

    fn hit(request: &str, path: &str, sha: &str) -> FontMatch {
        FontMatch {
            request: request.to_string(),
            bold: false,
            italic: false,
            chars: "你".to_string(),
            pen: 1,
            sources: vec![],
            verdict: Verdict::Exact,
            face: Some(FaceSummary {
                family: request.to_string(),
                style_name: "Regular".to_string(),
                weight: 400,
                italic: false,
                width: 5,
                file: path.to_string(),
                path: path.to_string(),
                face_index: 0,
                collection_len: 1,
                match_by: "family".to_string(),
                variable: false,
                source_sha256: sha.to_string(),
            }),
            candidates: vec![],
            missing_glyphs: String::new(),
            why: String::new(),
        }
    }

    fn miss(request: &str) -> FontMatch {
        let mut m = hit(request, "none", "none");
        m.face = None;
        m.verdict = Verdict::Missing;
        m
    }

    /// 核心用例：字体内容变了（哈希变了）就必须换身份。这是补这条之前会漏的场景。
    #[test]
    fn a_changed_font_file_changes_the_identity() {
        let db = db_with(&["C:\\Windows\\Fonts"]);
        let before = identify(&db, &[hit("Arial", "arial.ttf", "aaa")], &[]);
        let after = identify(&db, &[hit("Arial", "arial.ttf", "bbb")], &[]);
        assert_ne!(
            before.digest, after.digest,
            "字体升级后复用旧像素就是假交付"
        );
        assert_eq!(before.faces_used, 1);
        assert_eq!(before.faces_missing, 0);
    }

    /// 装上一个原本缺席的字体，也必须换身份 —— 缺席本身是环境的一部分。
    #[test]
    fn a_request_that_later_resolves_changes_the_identity() {
        let db = db_with(&["D"]);
        let gone = identify(&db, &[miss("FooSans")], &[]);
        let here = identify(&db, &[hit("FooSans", "foo.ttf", "abc")], &[]);
        assert_ne!(gone.digest, here.digest);
        assert_eq!(gone.faces_missing, 1);
        assert_eq!(here.faces_used, 1);
    }

    #[test]
    fn roots_are_part_of_the_identity_even_with_the_same_faces() {
        let a = identify(&db_with(&["X"]), &[hit("Arial", "a.ttf", "s")], &[]);
        let b = identify(&db_with(&["Y"]), &[hit("Arial", "a.ttf", "s")], &[]);
        assert_ne!(a.digest, b.digest, "--fonts-dir 指到别处就是另一个环境");
        // policy 是规则版本，不随来源变化；变化的应该是摘要。这条断言写反过一次（AI6）
        assert_eq!(a.policy, POLICY_VERSION);
        assert_eq!(a.policy, b.policy);
        assert!(
            a.entries.iter().any(|e| e == "root|X") && b.entries.iter().any(|e| e == "root|Y"),
            "来源本身必须留在报告里，不然换个目录为什么失效就看不出来：{:?}",
            a.entries
        );
    }

    /// 同一格内容换顺序不能换身份：摘要必须与枚举顺序无关（否则两次扫描可能自己否定自己）
    #[test]
    fn identity_is_order_independent() {
        let db = db_with(&["R"]);
        let one = identify(
            &db,
            &[
                hit("Arial", "a.ttf", "1"),
                hit("Impact", "i.ttf", "2"),
                miss("Zzz"),
            ],
            &[],
        );
        let other = identify(
            &db,
            &[
                miss("Zzz"),
                hit("Impact", "i.ttf", "2"),
                hit("Arial", "a.ttf", "1"),
            ],
            &[],
        );
        assert_eq!(one.digest, other.digest);
        assert_eq!(one.entries, other.entries);
    }

    /// 家族名里的多余空白是同一份请求，不该造出两个身份。
    #[test]
    fn whitespace_in_a_family_name_does_not_split_the_identity() {
        let db = db_with(&["R"]);
        let a = identify(&db, &[hit("Arial  Black", "a.ttf", "1")], &[]);
        let b = identify(&db, &[hit("Arial Black", "a.ttf", "1")], &[]);
        assert_eq!(a.digest, b.digest, "{a:?} {b:?}");
        let _ = req("x");
    }

    /// "这次是不是吃了缓存"是花了多少钱的事实，不是环境事实：同一份环境在冷启动与
    /// 缓存命中两次必须给出**同一个**摘要，否则缓存本身就会让渲染缓存反复失效。
    /// 这一条是 `evaluate_or_rescan` 那套"只慢不错"设计的另一半。
    #[test]
    fn a_cache_hit_and_a_full_rescan_give_the_same_identity() {
        let mut db = db_with(&["R"]);
        let plain = identify(&db, &[hit("Arial", "a.ttf", "1")], &[]);
        db.cache = CacheState::Hit;
        let from_hit = identify(&db, &[hit("Arial", "a.ttf", "1")], &[]);
        db.cache = CacheState::Refreshed;
        let refreshed = identify(&db, &[hit("Arial", "a.ttf", "1")], &[]);
        assert_eq!(plain.digest, from_hit.digest);
        assert_eq!(plain.digest, refreshed.digest, "{plain:?} {refreshed:?}");
        // 但报告里得看得出来的确走了不同的路
        assert_eq!(from_hit.cache, CacheState::Hit);
        assert_eq!(refreshed.cache, CacheState::Refreshed);
    }
}
