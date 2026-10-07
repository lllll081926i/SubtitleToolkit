//! 输入发现（15 §2）：文件列表 / 目录 / 递归 / include-exclude glob /
//! BDN 配对（IDX+SUB）/ 可选的同名视频关联。
//!
//! 三条刻意的设计：
//! 1. **不引第三方 glob**。15 §2 只要 `*` 与 `?` 这类通配，手写匹配器 60 行就能覆盖，
//!    而为一个模式匹配拉 `globset`（21 §1）需要单独审批（ISSUES E6）。
//!    审批没下来之前，缺依赖不是把功能做错的理由，也不是把它省掉的理由。
//! 2. **结果必须确定**：同一目录树两次发现得到同样的顺序（排序 + 去重），
//!    否则批量任务的 golden 与缓存 key 都会漂。
//! 3. **读不动的东西记账而不是消失**：没权限、目录损坏、孤立的 `.idx`，
//!    都进 `skipped` 并给出原因。批量报告要能回答"为什么我的文件没被处理"（15 §8）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};

/// BDN 目录结构里的成对文件（15 §2：paired files IDX+SUB）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub idx: PathBuf,
    /// 与 IDX 同名的 SUP；`None` 表示这组配对不完整，已进 `skipped`
    pub sub: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    /// 主输入：字幕/位图文件，按路径排序、去重
    pub files: Vec<PathBuf>,
    pub pairs: Vec<Pair>,
    /// 字幕 -> 同名视频（15 §2 的 optional video association，16 § 用它取帧率候选）
    pub videos: Vec<(PathBuf, PathBuf)>,
    /// 被排除或读不动的东西，带原因
    pub skipped: Vec<String>,
}

impl Discovered {
    pub fn primary_sha_hint(&self) -> Option<&Path> {
        self.files.first().map(|p| p.as_path())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverOptions {
    pub roots: Vec<PathBuf>,
    pub recursive: bool,
    /// 小写模式串，例如 `*.ass`；空列表表示不按 include 过滤
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub video_association: bool,
}

impl Default for DiscoverOptions {
    fn default() -> DiscoverOptions {
        DiscoverOptions {
            roots: Vec::new(),
            recursive: true,
            include: Vec::new(),
            exclude: Vec::new(),
            video_association: true,
        }
    }
}

/// 认得的字幕/位图扩展名。不在这里出现的名字不会被当成主输入。
pub const SUBTITLE_EXTS: &[&str] = &["ass", "ssa", "srt", "vtt", "sup", "idx", "xml"];
pub const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "m4v", "ts", "m2ts", "mov", "avi", "webm"];

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 通配匹配：`*` 任意串、`?` 单字符，其余按字面（大小写不敏感，Windows 语义）。
/// 迭代式实现，不递归回溯，避免模式里多个 `*` 时栈爆炸。
pub fn matches(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let txt: Vec<char> = name.to_ascii_lowercase().chars().collect();
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star: Option<usize> = None;
    let mut star_t = 0usize;
    while t < txt.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == txt[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some(p);
            star_t = t;
            p += 1;
        } else if let Some(sp) = star {
            // 回退到 `*` 的下一个候选：把多一个字符交给 `*`
            p = sp + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

fn any_match(pats: &[String], name: &str) -> bool {
    pats.iter().any(|pat| matches(pat, name))
}

impl DiscoverOptions {
    pub fn validate(&self) -> Result<()> {
        if self.roots.is_empty() {
            return Err(StkError::simple(
                Domain::Input,
                84,
                "输入发现没有任何根路径",
            ));
        }
        for pat in self.include.iter().chain(self.exclude.iter()) {
            if pat.is_empty() {
                return Err(
                    StkError::simple(Domain::Input, 85, "include/exclude 里有空模式")
                        .with_action("空模式等于匹配一切，会让批量任务静默吞掉整个目录"),
                );
            }
        }
        Ok(())
    }

    /// 单个候选是否入选：include 为空时全部接受，exclude 永远有否决权。
    fn accepts(&self, name: &str) -> Option<String> {
        if !self.exclude.is_empty() && any_match(&self.exclude, name) {
            return Some("命中 exclude".to_string());
        }
        if !self.include.is_empty() && !any_match(&self.include, name) {
            return Some("不在 include 里".to_string());
        }
        None
    }

    pub fn discover(&self) -> Result<Discovered> {
        self.validate()?;
        let mut found: BTreeSet<PathBuf> = BTreeSet::new();
        let mut skipped: Vec<String> = Vec::new();
        // 成员判断用集合：下面配对与视频关联都要反复查，向量线性扫描既慢，
        // 又让 contains/iter().any 两种写法在 clippy 里来回改
        let mut all_seen: BTreeSet<PathBuf> = BTreeSet::new();

        for root in &self.roots {
            if !root.exists() {
                return Err(StkError::simple(Domain::Input, 86, "输入根路径不存在")
                    .with_detail(root.display().to_string()));
            }
            if root.is_file() {
                let name = root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                all_seen.insert(root.clone());
                match self.accepts(&name) {
                    None => {
                        found.insert(root.clone());
                    }
                    Some(why) => skipped.push(format!("{}：{why}", root.display())),
                }
                continue;
            }
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let rd = match std::fs::read_dir(&dir) {
                    Ok(rd) => rd,
                    Err(e) => {
                        // 读不动的目录不能"当不存在"，那会让用户以为文件没被扫到
                        skipped.push(format!("{}：目录读取失败 {e}", dir.display()));
                        continue;
                    }
                };
                let mut entries: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file() || p.is_dir())
                    .collect();
                entries.sort();
                for p in entries {
                    if p.is_dir() {
                        if self.recursive {
                            stack.push(p);
                        }
                        continue;
                    }
                    all_seen.insert(p.clone());
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let ext = ext_of(&p);
                    if self.accepts(&name).is_some() {
                        skipped.push(format!("{}：被 include/exclude 排除", p.display()));
                        continue;
                    }
                    if SUBTITLE_EXTS.contains(&ext.as_str()) {
                        found.insert(p);
                    } else {
                        skipped.push(format!("{}：扩展名不是可处理的字幕/位图", p.display()));
                    }
                }
            }
        }

        let files: Vec<PathBuf> = found.into_iter().collect();
        let mut pairs = Vec::new();
        for f in &files {
            if ext_of(f) != "idx" {
                continue;
            }
            let stem = f.with_extension("");
            let sub = ["sup", "sm2asset"]
                .iter()
                .map(|e| stem.with_extension(e))
                .find(|p| all_seen.contains(p));
            if sub.is_none() {
                skipped.push(format!("{}：IDX 没有同名 SUB/SUP 配对", f.display()));
            }
            pairs.push(Pair {
                idx: f.clone(),
                sub,
            });
        }
        pairs.sort_by(|a, b| a.idx.cmp(&b.idx));

        let mut videos = Vec::new();
        if self.video_association {
            for f in &files {
                let e = ext_of(f);
                if !matches!(e.as_str(), "ass" | "ssa" | "srt" | "vtt") {
                    continue;
                }
                let stem = f.with_extension("");
                for ve in VIDEO_EXTS {
                    let cand = stem.with_extension(ve);
                    if all_seen.contains(&cand) {
                        videos.push((f.clone(), cand));
                        break;
                    }
                }
            }
            videos.sort();
        }

        Ok(Discovered {
            files,
            pairs,
            videos,
            skipped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-discover-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(dir: &Path, rel: &str) -> PathBuf {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, b"content").unwrap();
        p
    }

    #[test]
    fn wildcard_matcher_is_exact_about_star_question_and_case() {
        assert!(matches("*.ass", "a.ass"));
        assert!(
            matches("*.ass", "A.ASS"),
            "Windows 上文件名比较不区分大小写"
        );
        assert!(!matches("*.ass", "a.srt"));
        assert!(matches("a?.srt", "ab.srt"));
        assert!(!matches("a?.srt", "abc.srt"));
        assert!(matches("*.mid*.ass", "a.middle.b.ass"), "多个 * 必须能回溯");
        assert!(!matches("**.ass", "a.srt"));
        assert!(
            matches("", ""),
            "空模式只匹配空名——空模式在 validate 里就被拒绝"
        );
    }

    #[test]
    fn discovery_is_recursive_sorted_and_deduplicated() {
        let dir = tmp("rec");
        touch(&dir, "b.ass");
        touch(&dir, "sub/a.ass");
        touch(&dir, "sub/deep/c.srt");
        touch(&dir, "notes.txt");
        let opt = DiscoverOptions {
            roots: vec![dir.clone()],
            ..Default::default()
        };
        let d = opt.discover().unwrap();
        let names: Vec<String> = d
            .files
            .iter()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            names,
            vec!["b.ass", "sub/a.ass", "sub/deep/c.srt"],
            "必须排序且只含可处理扩展名：{names:?}"
        );
        assert!(
            d.skipped.iter().any(|s| s.contains("notes.txt")),
            "被跳过的文件要能解释：{:?}",
            d.skipped
        );
        let again = opt.discover().unwrap();
        assert_eq!(d, again, "同一目录树两次发现必须逐字节相同");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_recursive_skips_nested_directories() {
        let dir = tmp("nonrec");
        touch(&dir, "a.ass");
        touch(&dir, "sub/b.ass");
        let opt = DiscoverOptions {
            roots: vec![dir.clone()],
            recursive: false,
            ..Default::default()
        };
        let d = opt.discover().unwrap();
        assert_eq!(d.files.len(), 1, "{:?}", d.files);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn include_and_exclude_have_the_documented_precedence() {
        let dir = tmp("filters");
        touch(&dir, "keep.ass");
        touch(&dir, "drop.srt");
        touch(&dir, "backup.ass.bak");
        let opt = DiscoverOptions {
            roots: vec![dir.clone()],
            include: vec!["*.ass".into()],
            exclude: vec!["*drop*".into()],
            ..Default::default()
        };
        let d = opt.discover().unwrap();
        let names: Vec<String> = d
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["keep.ass"], "exclude 有否决权：{names:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_pattern_is_refused_because_it_would_swallow_the_tree() {
        let dir = tmp("emptypat");
        touch(&dir, "a.ass");
        let opt = DiscoverOptions {
            roots: vec![dir.clone()],
            include: vec![String::new()],
            ..Default::default()
        };
        assert_eq!(opt.discover().unwrap_err().code_string(), "STK-INP-0085");
        let no_roots = DiscoverOptions::default();
        assert_eq!(
            no_roots.discover().unwrap_err().code_string(),
            "STK-INP-0084"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_root_is_an_error_not_a_silently_empty_result() {
        let opt = DiscoverOptions {
            roots: vec![PathBuf::from("Z:\\definitely-not-here-9x2")],
            ..Default::default()
        };
        assert_eq!(opt.discover().unwrap_err().code_string(), "STK-INP-0086");
    }

    #[test]
    fn idx_without_a_partner_is_recorded_not_dropped() {
        let dir = tmp("pairs");
        touch(&dir, "movie.idx");
        touch(&dir, "movie.sup");
        touch(&dir, "lonely.idx");
        let opt = DiscoverOptions {
            roots: vec![dir.clone()],
            ..Default::default()
        };
        let d = opt.discover().unwrap();
        assert_eq!(d.pairs.len(), 2, "IDX 都要出现在配对表里：{:?}", d.pairs);
        let paired = d
            .pairs
            .iter()
            .find(|p| p.idx == dir.join("movie.idx"))
            .unwrap();
        assert!(paired.sub.is_some(), "同名 SUP 必须配上对");
        let lonely = d
            .pairs
            .iter()
            .find(|p| p.idx == dir.join("lonely.idx"))
            .unwrap();
        assert_eq!(lonely.sub, None);
        assert!(
            d.skipped
                .iter()
                .any(|s| s.contains("lonely.idx") && s.contains("配对")),
            "孤立 IDX 要能被解释：{:?}",
            d.skipped
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_name_video_is_associated_and_can_be_turned_off() {
        let dir = tmp("video");
        let ass = touch(&dir, "movie.ass");
        touch(&dir, "movie.mkv");
        touch(&dir, "other.srt");
        let d = opt(&dir, true).discover().unwrap();
        assert_eq!(d.videos.len(), 1, "{:?}", d.videos);
        assert_eq!(d.videos[0].0, ass);
        assert_eq!(
            d.videos[0].1,
            dir.join("movie.mkv"),
            "只关联同名视频（16 §）"
        );
        assert!(opt(&dir, false).discover().unwrap().videos.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn opt(dir: &Path, video: bool) -> DiscoverOptions {
        DiscoverOptions {
            roots: vec![dir.to_path_buf()],
            video_association: video,
            ..Default::default()
        }
    }
}
