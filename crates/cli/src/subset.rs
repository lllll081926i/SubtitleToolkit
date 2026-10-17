//! `font subset`（17 §5/§6）：把 ASS 真正用到的字符集做成字体子集。
//!
//! ## 这个命令的诚实边界
//!
//! 17 §5 要求的完整等价语料（CJK+IVS、Arabic joining、Indic、combining、vertical、
//! color font、variable、CFF/CFF2、TTC/OTC、composite）**尚未建立**，
//! 所以本命令做的是：
//!
//! 1. 复杂脚本一律回退整份（17 §5 最后一句的硬性要求）；
//! 2. 其余字符做 cmap 级等价检查（字符解出的字形存在性与 advance 必须一致）；
//! 3. **不宣称渲染等价** —— 那需要真 renderer + 授权语料，报告 `scope` 里明说。
//!
//! ## 为什么默认不改写字体、而是另开目录
//!
//! 17 §8 第一条："subset 是可回退优化"，第三条："不覆盖用户原字体"。
//! 所以子集写到 `--subset-dir`（默认 `<原字体所在目录>/.subtitletool-subset/`），
//! 并且 manifest 里同时记原字体哈希与子集哈希，任何时刻都能回到原字体。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};
use font_service::report::{FontMatch, Verdict};
use font_service::subset::{self, FallbackReason, Outcome};

use crate::args::{Command, Invocation};
use crate::output::{Envelope, WriteRecord};
use crate::pipeline;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

/// 子集请求：一份字体文件 + 它要保留的字符集。
struct Request {
    /// 源文件（绝对路径字符串，来自 FontDb 记录）
    path: String,
    face_index: u32,
    /// 该 face 被哪些 ASS 请求引用（报告用）
    requested_by: Vec<String>,
    chars: BTreeSet<char>,
}

pub fn run_subset(inv: &Invocation) -> Result<Envelope> {
    let cmd = inv.command.name();
    let Command::FontSubset {
        input,
        fonts_dirs,
        registry,
        subset_dir,
        fallback_full,
        embed_fonts,
        mkv_attachments,
    } = &inv.command
    else {
        return Err(cli_err(280, "font 命令分派错误"));
    };

    // 与 font scan 完全同一套"抽取用量 → 建库 → 评级"：子集必须建立在
    // 同一份用量结论上，否则"要保留哪些字"会在两个命令之间漂移。
    let loaded = pipeline::load_text(input)?;
    let requests = font_service::usage::scan_text(&loaded.text)?;

    let dirs = if fonts_dirs.is_empty() {
        crate::font::default_font_dirs()
    } else {
        let mut d = fonts_dirs.clone();
        d.sort();
        d.dedup();
        d
    };
    if dirs.is_empty() {
        return Err(cli_err(281, "找不到任何字体目录，无法做子集")
            .with_action("用 --fonts-dir DIR 指明字体库位置（可重复）"));
    }
    let missing_dir: Vec<String> = dirs
        .iter()
        .filter(|d| !d.is_dir())
        .map(|d| d.display().to_string())
        .collect();
    if !missing_dir.is_empty() {
        return Err(cli_err(282, "指定的字体目录不存在").with_detail(missing_dir.join("；")));
    }

    let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
    let reg = if *registry {
        font_service::registry::enumerate()
    } else {
        font_service::registry::RegistryReport::default()
    };
    let mode = crate::fontenv::cache_mode(inv);
    let mut db = font_service::db::FontDb::scan_cached(&refs, &reg.font_files(), mode);
    let matches = font_service::evaluate_or_rescan(&mut db, &refs, &reg.font_files(), &requests)?;

    let plan = build_plan(&matches)?;
    if plan.is_empty() {
        return Err(cli_err(283, "没有任何解析到的 face 可以做子集")
            .with_action("先跑 font scan 看 verdict=missing 的条目"));
    }

    let dest = match subset_dir {
        Some(d) => {
            if !d.is_absolute() {
                return Err(
                    cli_err(284, "--subset-dir 需要绝对路径").with_detail(d.display().to_string())
                );
            }
            d.clone()
        }
        None => default_subset_dir(&plan)?,
    };
    if !inv.dry_run {
        std::fs::create_dir_all(&dest).map_err(|e| {
            cli_err(285, "无法创建子集输出目录").with_detail(format!("{}：{e}", dest.display()))
        })?;
    }

    let mut writes: Vec<WriteRecord> = Vec::new();
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut total_source = 0usize;
    let mut total_output = 0usize;
    let mut fallbacks: Vec<serde_json::Value> = Vec::new();
    // 两档新输出要用到"最终交付的字节"（可能是子集，也可能是整份回退），
    // 所以在循环里存下来，而不是到后面再去读文件——读了就可能读到刚被别人换掉的版本。
    let mut delivered: Vec<(String, Vec<u8>)> = Vec::new();

    for req in &plan {
        let src = Path::new(&req.path);
        let bytes = std::fs::read(src).map_err(|e| {
            cli_err(286, "读不到要子集的字体文件").with_detail(format!("{}：{e}", src.display()))
        })?;
        let outcome = subset::subset_bytes(&bytes, req.face_index, &req.chars)?;
        *counts
            .entry(outcome.outcome.label().to_string())
            .or_insert(0) += 1;
        total_source += outcome.source_bytes;
        total_output += outcome.bytes.len();

        // `--fallback-full`：只要有一份回退就以失败退出。
        // 存在理由是流水线需要"要么全是子集、要么别交付"的开关，
        // 而不是让调用方去解析 JSON 才知道发生了什么。
        if *fallback_full {
            if let Some(reason) = &outcome.fallback {
                fallbacks.push(serde_json::json!({
                    "source_file": req.path,
                    "reason": reason.code(),
                    "detail": reason.describe(),
                }));
            }
        }

        let name = subset::subset_path(&dest, src, &outcome.source_sha256);
        // 附件名用原子字体文件名：mkv 附件与 ASS `[Fonts]` 都是按名字被人看的，
        // `<stem>.<sha8>.subset.ttf` 这种名字在播放器里会很难认。
        let attach_name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "font.ttf".to_string());
        delivered.push((attach_name.clone(), outcome.bytes.clone()));
        let verify_sha = outcome.output_sha256.clone();
        let verify_parseable = outcome.outcome == Outcome::Subset;
        let emitted = pipeline::emit(
            Some(&name),
            &name,
            &outcome.bytes,
            move |b| {
                let got = pipeline::sha256_hex(b);
                if got != verify_sha {
                    return Err(cli_err(287, "写出的字体与计算结果不一致"));
                }
                // 子集产物必须自己也能被解析器读回来：交付一个坏字体是最坏的失败
                if verify_parseable && !font_service::is_parseable_font(b) {
                    return Err(cli_err(288, "子集产物无法被解析器读回"));
                }
                Ok(())
            },
            inv.dry_run,
            inv.overwrite,
        )?;
        let note = emitted.describe();
        writes.push(WriteRecord {
            path: note.path,
            bytes: note.bytes,
            state: note.state,
        });

        entries.push(serde_json::json!({
            "source_file": req.path,
            "face_index": req.face_index,
            "requested_by": req.requested_by,
            "chars": req.chars.len(),
            "source_sha256": outcome.source_sha256,
            "source_bytes": outcome.source_bytes,
            "subset_sha256": outcome.output_sha256,
            "output_bytes": outcome.bytes.len(),
            "delivered_as": name.file_name().map(|s| s.to_string_lossy().into_owned()),
            "attachment_name": attach_name,
            "outcome": outcome.outcome,
            "fallback_reason": outcome.fallback.as_ref().map(|r| r.code()),
            "fallback_detail": outcome.fallback.as_ref().map(|r| r.describe()),
            "equivalence_probes": outcome.equivalence_probes,
            "subset_glyphs": outcome.subset_glyphs,
            "saved_bytes": outcome.saved_bytes(),
        }));
    }

    let manifest_path = dest.join("subset-manifest.json");
    let manifest = serde_json::json!({
        "schema": 1,
        "mode": "copy subset fonts",
        "ass_input": input.display().to_string(),
        "delivered_dir": dest.display().to_string(),
        "fonts": entries,
        "outcome_counts": counts,
        "total_source_bytes": total_source,
        "total_output_bytes": total_output,
        "total_saved_bytes": total_source as i64 - total_output as i64,
        "scope": [
            "复杂脚本（Arabic/Indic/combining/vertical/IVS/emoji）一律回退整份：17 §5 的硬性要求",
            "等价检查为 cmap 级：字符解出的字形存在性与水平 advance 必须与原字体一致",
            "未做渲染级等价（RGBA 对照）：17 §5 的 corpus 与真 renderer 尚未接入，不代表渲染必然一致",
            "合字（如 fi）在逐字符检查里看不出来，可能被漏过",
            "TTC/OTC 成员与 variable font 未覆盖，遇到即回退整份",
        ],
        "limits": [
            "子集是优化不是必需：任何一份回退都说明那一份按原样交付",
            "manifest 里原字体 hash 与子集 hash 都在，任何时刻都能回到原字体（17 §8）",
            "不覆盖用户原字体：产物写在 --subset-dir（默认原字体目录下的隐藏子目录）",
        ],
    });
    let pretty = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| cli_err(289, "子集 manifest 序列化失败").with_detail(e.to_string()))?;
    let emitted = pipeline::emit(
        Some(&manifest_path),
        &manifest_path,
        &pretty,
        |b| {
            serde_json::from_slice::<serde_json::Value>(b)
                .map(|_| ())
                .map_err(|e| cli_err(290, "子集 manifest 不是合法 JSON").with_detail(e.to_string()))
        },
        inv.dry_run,
        inv.overwrite,
    )?;
    let note = emitted.describe();
    writes.push(WriteRecord {
        path: note.path,
        bytes: note.bytes,
        state: note.state,
    });

    if *fallback_full && !fallbacks.is_empty() {
        let e = cli_err(291, "有字体无法安全子集，按 --fallback-full 整体拒绝交付")
            .with_detail(
                fallbacks
                    .iter()
                    .map(|f| {
                        format!(
                            "{}：{}",
                            f["source_file"].as_str().unwrap_or(""),
                            f["detail"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("；"),
            )
            .with_action("去掉 --fallback-full 就会把那些字体整份交付");
        return Err(e);
    }

    // 17 §6 第四/五档：两档都基于“最终交付的字节”（子集或整份回退），
    // 所以必须在同一份 `delivered` 上做，不能各自再去读一次磁盘。
    let mut extra_outputs: Vec<serde_json::Value> = Vec::new();
    if *embed_fonts {
        let out = crate::deliver::embed_fonts(inv, &loaded.text, input, &delivered, &dest)?;
        writes.extend(out.writes);
        extra_outputs.push(out.report);
    }
    if *mkv_attachments {
        let out = crate::deliver::mkv_attachments(inv, &delivered, &dest)?;
        writes.extend(out.writes);
        extra_outputs.push(out.report);
    }

    let mut env = Envelope::ok(
        cmd,
        serde_json::json!({
            "ass_input": input.display().to_string(),
            "delivered_dir": dest.display().to_string(),
            "manifest": manifest,
            "outputs": extra_outputs,
        }),
    );
    env.writes = writes;
    Ok(env)
}

/// 把评级结果收敛成"每份字体要保留哪些字符"。
///
/// 同一份字体文件可能被多条 ASS 请求命中（常规/粗体、多个 style），
/// 合并成一条请求：对同一份文件重复做子集既慢又会互相覆盖输出名。
fn build_plan(matches: &[FontMatch]) -> Result<Vec<Request>> {
    let mut by_file: BTreeMap<String, Request> = BTreeMap::new();
    for m in matches {
        let Some(face) = &m.face else { continue };
        // 只有真的解析到文件的条目才有字节可子集
        if m.verdict == Verdict::Missing {
            continue;
        }
        let entry = by_file.entry(face.path.clone()).or_insert_with(|| Request {
            path: face.path.clone(),
            face_index: face.face_index,
            requested_by: Vec::new(),
            chars: BTreeSet::new(),
        });
        for c in m.chars.chars() {
            entry.chars.insert(c);
        }
        let mut tags = String::new();
        if m.bold {
            tags.push('B');
        }
        if m.italic {
            tags.push('I');
        }
        let who = format!("{}（{tags}）", m.request);
        if !entry.requested_by.contains(&who) {
            entry.requested_by.push(who);
        }
    }
    Ok(by_file.into_values().collect())
}

/// 默认输出目录：源字体目录下的隐藏子目录。
///
/// 不用系统临时目录：子集产物是要交付给 MKV 附件或 ASS [Fonts] 的，
/// 放在临时目录里会被清理掉，用户只会看到"交付了但找不到"。
fn default_subset_dir(plan: &[Request]) -> Result<PathBuf> {
    let first = plan
        .first()
        .ok_or_else(|| cli_err(292, "没有可子集的字体"))?;
    let parent = Path::new(&first.path)
        .parent()
        .ok_or_else(|| {
            cli_err(293, "字体文件没有父目录，无法决定子集输出位置").with_detail(first.path.clone())
        })?
        .to_path_buf();
    Ok(parent.join(".subtitletool-subset"))
}

/// 给报告用的说明：某个回退原因意味着什么。
pub fn explain(reason: &FallbackReason) -> String {
    reason.describe()
}

#[cfg(test)]
mod tests {
    use super::*;
    use font_service::report::{FaceSummary, Verdict};

    fn m(path: &str, idx: u32, chars: &str, bold: bool, verdict: Verdict) -> FontMatch {
        FontMatch {
            request: "Default".to_string(),
            bold,
            italic: false,
            verdict,
            face: Some(FaceSummary {
                path: path.to_string(),
                face_index: idx,
                collection_len: 1,
                family: "X".to_string(),
                style_name: "Normal".to_string(),
                weight: 400,
                italic: false,
                match_by: "family".to_string(),
                source_sha256: "aa".to_string(),
                file: path.to_string(),
                variable: false,
                width: 5,
            }),
            missing_glyphs: String::new(),
            chars: chars.to_string(),
            sources: vec![],
            pen: 0,
            candidates: vec![],
            why: String::new(),
        }
    }

    #[test]
    fn plan_merges_requests_for_the_same_file() {
        let plan = build_plan(&[
            m("D:\\f.ttf", 0, "abc", false, Verdict::Exact),
            m("D:\\f.ttf", 0, "cde", true, Verdict::Exact),
            m("D:\\g.ttf", 0, "xyz", false, Verdict::Exact),
        ])
        .unwrap();
        assert_eq!(plan.len(), 2, "同一文件的多条请求必须合并");
        let f = plan.iter().find(|r| r.path.ends_with("f.ttf")).unwrap();
        assert_eq!(
            f.chars.iter().collect::<String>(),
            "abcde",
            "字符集要并起来"
        );
        assert_eq!(f.requested_by.len(), 2, "两条来源都要留下");
    }

    #[test]
    fn plan_skips_missing_so_no_bogus_bytes_are_produced() {
        let plan = build_plan(&[
            m("D:\\f.ttf", 0, "abc", false, Verdict::Missing),
            m("D:\\g.ttf", 0, "abc", false, Verdict::Exact),
        ])
        .unwrap();
        assert_eq!(plan.len(), 1, "missing 的条目没有文件，不能进计划");
        assert!(plan[0].path.ends_with("g.ttf"));
    }

    #[test]
    fn default_dir_is_next_to_the_source_font_not_in_temp() {
        let plan = build_plan(&[m("D:\\fonts\\f.ttf", 0, "a", false, Verdict::Exact)]).unwrap();
        let d = default_subset_dir(&plan).unwrap();
        assert_eq!(d, PathBuf::from("D:\\fonts\\.subtitletool-subset"));
    }

    #[test]
    fn empty_plan_is_an_error_not_a_silent_success() {
        assert!(build_plan(&[]).unwrap().is_empty());
        assert!(default_subset_dir(&[]).is_err());
    }
}
