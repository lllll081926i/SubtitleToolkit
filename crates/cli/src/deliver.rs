//! 17 §6 的第二/五档：复制整份字体 + manifest。
//!
//! 只做"复制整份"这一档：**不 subset**（17 §5 的等价 corpus 没建立，ISSUES AG2），
//! 所以 manifest 里没有 "原 hash → subset hash" 的那一半，这一点写在 `limits` 里。
//!
//! 17 §8 的安全策略在这里是硬要求：
//! * "永远保留原字体来源信息"——每条交付都记源文件路径与 SHA-256；
//! * 复制出去的字节必须还能被解析器读回来（判据来自解析器本身，不是文件大小相同）；
//! * 走 `pipeline::emit`：原子写、覆盖策略、`--dry-run` 不碰盘，与其它产物完全同一套规矩。

use std::collections::BTreeSet;
use std::path::Path;

use core_model::error::{Domain, Result, StkError};
use font_service::report::FontMatch;

use crate::args::Invocation;
use crate::output::WriteRecord;
use crate::pipeline;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Output, code, msg)
}

fn digest(bytes: &[u8]) -> String {
    crate::pipeline::sha256_hex(bytes)
}

/// 目标文件名：保留原文件名（含扩展名），集合成员共用同一个文件、由 manifest 记 index。
fn target_name(src: &Path) -> Result<String> {
    src.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            cli_err(190, "字体文件名无法用于交付").with_detail(src.display().to_string())
        })
}

/// 把 `matches` 里解析到的 face 文件复制到 `dest`，并写 `manifest.json`。
///
/// 返回写盘记录 + manifest（manifest 同时也进 CLI 的信封，让脚本不读盘也能判断）。
pub fn copy_fonts(
    inv: &Invocation,
    matches: &[FontMatch],
    dest: &Path,
) -> Result<(Vec<WriteRecord>, serde_json::Value)> {
    if !dest.is_absolute() {
        return Err(cli_err(191, "交付目录必须是绝对路径").with_detail(dest.display().to_string()));
    }
    // 同一个文件可能被多条请求命中（TTC 里两个成员、或常规/粗体指到同一文件）：只复制一次
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut picked: Vec<&FontMatch> = Vec::new();
    for m in matches {
        let Some(face) = &m.face else { continue };
        if seen.insert(face.path.clone()) {
            picked.push(m);
        }
    }
    if picked.is_empty() {
        return Err(
            cli_err(192, "没有任何字体可以交付：所有请求都没有解析到的 face")
                .with_action("先看 verdict=missing 的条目，或加 --fonts-dir 指明字体所在目录"),
        );
    }
    if !inv.dry_run {
        std::fs::create_dir_all(dest).map_err(|e| {
            cli_err(193, "无法创建字体交付目录").with_detail(format!("{}：{e}", dest.display()))
        })?;
    }
    let mut writes: Vec<WriteRecord> = Vec::new();
    let mut entries: Vec<serde_json::Value> = Vec::new();

    for m in picked {
        let face = m.face.as_ref().expect("picked 来自有 face 的匹配");
        let src = Path::new(&face.path);
        let bytes = std::fs::read(src).map_err(|e| {
            cli_err(194, "读不到要复制的字体文件").with_detail(format!("{}：{e}", src.display()))
        })?;
        let src_sha = digest(&bytes);
        if !face.source_sha256.is_empty() && face.source_sha256 != src_sha {
            // 扫描与交付之间字体被换掉了（升级、覆盖安装）。这时不能"拿新字节的哈希
            // 当旧结论用"——那份报告说的是另一个文件（AT-FONT-002 的同一条纪律）
            return Err(cli_err(200, "字体文件在扫描之后被换掉了，拒绝交付")
                .with_detail(format!(
                    "{}：扫描时 {前}，现在 {后}",
                    src.display(),
                    前 = face.source_sha256,
                    后 = src_sha
                ))
                .with_action("重跑 font scan 取得新身份，再决定交付哪些字体"));
        }
        if !font_service::is_parseable_font(&bytes) {
            return Err(cli_err(195, "字体文件读得出字节但解析器不认")
                .with_detail(format!("{}（sha256 {src_sha}）", src.display())));
        }
        let name = target_name(src)?;
        let target = dest.join(&name);
        let verify = |b: &[u8]| -> Result<()> {
            // 两条都要：字节完全一致（含哈希），且能被解析器读回来
            if digest(b) != src_sha {
                return Err(cli_err(196, "复制出去的字体与原文件不一致").with_detail(name.clone()));
            }
            if !font_service::is_parseable_font(b) {
                return Err(cli_err(197, "复制出去的字体无法解析").with_detail(name.clone()));
            }
            Ok(())
        };
        let emitted = pipeline::emit(
            Some(&target),
            &target,
            &bytes,
            verify,
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
            "request": m.request,
            "bold": m.bold,
            "italic": m.italic,
            "verdict": m.verdict,
            "matched_by": face.match_by,
            "family": face.family,
            "style_name": face.style_name,
            "weight": face.weight,
            "italic_face": face.italic,
            "source_file": face.path,
            "source_sha256": src_sha,
            "face_index": face.face_index,
            "collection_len": face.collection_len,
            "delivered_as": name,
            "bytes": bytes.len(),
            "chars": m.chars,
            "requested_by": m.sources,
            // 17 §6 的 manifest 要 "原 font hash → subset hash → ASS request"；
            // 没有 subset 就没有 subset hash，这一格明确留空而不是省略
            "subset_sha256": null,
        }));
    }

    let manifest = serde_json::json!({
        "schema": 1,
        "mode": "copy full fonts",
        "delivered_dir": dest.display().to_string(),
        "fonts": entries,
        "distinct_files": entries.len(),
        "limits": [
            "整份复制：没有 subset，因此 17 §6 的 \"subset hash\" 一格是 null（AG2）",
            "复制的是这台机器上解析到的那份文件；换机器或字体被更新后哈希会变（17 §8 记来源就是为了可比对）",
            "没有校验目标设备的渲染结果：manifest 说明的是\"给了哪些字节\"，不是\"渲染一定长成这样\"",
        ],
    });
    let pretty = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| cli_err(198, "manifest 序列化失败").with_detail(e.to_string()))?;
    let mpath = dest.join("manifest.json");
    let emitted = pipeline::emit(
        Some(&mpath),
        &mpath,
        &pretty,
        |b| {
            serde_json::from_slice::<serde_json::Value>(b)
                .map(|_| ())
                .map_err(|e| cli_err(199, "manifest 不是合法 JSON").with_detail(e.to_string()))
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
    Ok((writes, manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Command;
    use font_service::report::Verdict;
    use std::path::PathBuf;
    use task_engine::OverwritePolicy;

    fn face(path: &str, index: u32, len: u32) -> font_service::report::FaceSummary {
        font_service::report::FaceSummary {
            family: "Arial".to_string(),
            style_name: "Normal".to_string(),
            weight: 400,
            italic: false,
            width: 5,
            file: format!(
                "{}#{index}",
                path.rsplit(['/', '\\']).next().unwrap_or(path)
            ),
            path: path.to_string(),
            face_index: index,
            collection_len: len,
            match_by: "family".to_string(),
            variable: false,
            source_sha256: String::new(),
        }
    }

    fn match_with(path: &str, index: u32, len: u32) -> FontMatch {
        FontMatch {
            request: "Arial".to_string(),
            bold: false,
            italic: false,
            chars: "你好".to_string(),
            pen: 2,
            sources: vec!["style:Default".to_string()],
            verdict: Verdict::MissingGlyph,
            face: Some(face(path, index, len)),
            candidates: vec![],
            missing_glyphs: "你好".to_string(),
            why: "测试".to_string(),
        }
    }

    fn inv(dest: Option<PathBuf>, dry: bool) -> Invocation {
        Invocation {
            command: Command::FontScan {
                input: PathBuf::from("x.ass"),
                fonts_dirs: vec![],
                registry: true,
                copy_fonts: dest,
            },
            json: true,
            out: None,
            dry_run: dry,
            overwrite: OverwritePolicy::ReplaceAfterVerify,
            inject_fonts: false,
            no_font_cache: false,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-deliver-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 找一份真实字体：交付测的是"字节原样 + 还能读回来"，合成的假字节测不到这个。
    fn a_real_font(dir: &Path) -> Option<PathBuf> {
        let windir = std::env::var("WINDIR").ok()?;
        let sys = Path::new(&windir).join("Fonts");
        let rd = std::fs::read_dir(&sys).ok()?;
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|s| s.to_str()) == Some("ttf") && p.is_file() {
                let to = dir.join(p.file_name().unwrap());
                std::fs::copy(&p, &to).ok()?;
                return Some(to);
            }
        }
        None
    }

    #[test]
    fn relative_destination_is_refused_before_any_write() {
        let d = scratch("reldir");
        let m = match_with("C:\\Windows\\Fonts\\arial.ttf", 0, 1);
        let e = copy_fonts(
            &inv(Some(PathBuf::from("fonts-out")), false),
            &[m],
            Path::new("fonts-out"),
        )
        .unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0191", "{e:?}");
        std::fs::remove_dir_all(&d).ok();
    }

    /// 扫描之后字体被换掉 → 拒绝交付，而不是拿新字节的哈希继续用旧结论（AT-FONT-002 的纪律）
    #[test]
    fn a_font_replaced_after_the_scan_is_refused() {
        let d = scratch("stale");
        let Some(src) = a_real_font(&d) else {
            std::fs::remove_dir_all(&d).ok();
            panic!("本机没有 .ttf");
        };
        let out = d.join("deliver");
        let mut m = match_with(src.to_str().unwrap(), 0, 1);
        m.face.as_mut().unwrap().source_sha256 = "0".repeat(64);
        let e = copy_fonts(&inv(Some(out.clone()), false), &[m], &out).unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0200", "{e:?}");
        assert!(e.title.contains("换掉"), "{e}");
        assert!(
            !out.join("manifest.json").exists(),
            "拒绝交付就不该留下半个目录内容"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn nothing_is_delivered_when_no_request_resolved_a_face() {
        let mut m = match_with("C:\\Windows\\Fonts\\arial.ttf", 0, 1);
        m.face = None;
        let d = scratch("nofont");
        let e = copy_fonts(&inv(Some(d.clone()), false), &[m], &d).unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0192", "{e:?}");
        assert!(e.user_action.is_some(), "必须给出下一步：{e:?}");
        std::fs::remove_dir_all(&d).ok();
    }

    /// 整份复制的三条硬要求：字节一致、还能解析、manifest 记住来源。
    #[test]
    fn copied_bytes_match_the_source_and_are_still_parseable() {
        let d = scratch("copy");
        let Some(src) = a_real_font(&d) else {
            panic!("本机没有 .ttf 可复制，交付路径无法验证");
        };
        let bytes = std::fs::read(&src).unwrap();
        let want = digest(&bytes);
        let out = d.join("deliver");
        let m = match_with(src.to_str().unwrap(), 0, 1);
        let (writes, manifest) = copy_fonts(&inv(Some(out.clone()), false), &[m], &out).unwrap();
        assert_eq!(writes.len(), 2, "一份字体 + 一份 manifest：{writes:?}");
        assert!(writes.iter().all(|w| w.state == "written"), "{writes:?}");
        let copied = std::fs::read(out.join(src.file_name().unwrap())).unwrap();
        assert_eq!(digest(&copied), want, "复制出去的字节必须与源一致");
        assert!(font_service::is_parseable_font(&copied));
        let e = &manifest["fonts"][0];
        assert_eq!(e["source_sha256"], want, "17 §8：来源信息必须留档");
        assert_eq!(e["source_file"], src.to_str().unwrap());
        assert!(
            e["subset_sha256"].is_null(),
            "没做 subset 就不能假装有（AG2）"
        );
        assert_eq!(manifest["mode"], "copy full fonts");
        assert!(manifest["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("subset")));

        // dry-run 一个字节都不许落盘
        let dry = d.join("dry");
        let m2 = match_with(src.to_str().unwrap(), 0, 1);
        let (writes, _) = copy_fonts(&inv(Some(dry.clone()), true), &[m2], &dry).unwrap();
        assert!(
            writes.iter().all(|w| w.state == "would-write"),
            "{writes:?}"
        );
        assert!(!dry.join("manifest.json").exists(), "--dry-run 写了盘");
        std::fs::remove_dir_all(&d).ok();
    }

    /// 同一个文件被两条请求命中（TTC 的两个成员就是这种）：只复制一次，manifest 记 index。
    #[test]
    fn one_file_shared_by_two_members_is_copied_once() {
        let d = scratch("ttc");
        let windir = match std::env::var("WINDIR") {
            Ok(w) => w,
            Err(_) => {
                std::fs::remove_dir_all(&d).ok();
                return;
            }
        };
        let sys = Path::new(&windir).join("Fonts");
        let mut ttc: Option<PathBuf> = None;
        if let Ok(rd) = std::fs::read_dir(&sys) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|s| s.to_str()) == Some("ttc") && p.is_file() {
                    ttc = Some(p);
                    break;
                }
            }
        }
        let Some(ttc) = ttc else {
            eprintln!("SKIP：本机没有 .ttc");
            return;
        };
        let out = d.join("deliver");
        let a = match_with(ttc.to_str().unwrap(), 0, 3);
        let b = match_with(ttc.to_str().unwrap(), 1, 3);
        let (writes, manifest) = copy_fonts(&inv(Some(out.clone()), false), &[a, b], &out).unwrap();
        // 字体文件 1 份 + manifest 1 份
        assert_eq!(writes.len(), 2, "同一个文件不能复制两遍：{writes:?}");
        assert_eq!(manifest["fonts"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["fonts"][0]["face_index"], 0);
        std::fs::remove_dir_all(&d).ok();
    }
}
