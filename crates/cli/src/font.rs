//! `font scan`（17 §2–§4）：把一份 ASS 真正引用的 face 与本机字体库对上。
//!
//! 只做**报告**（17 §6 的第一档 "report only"）：不动任何文件、不装字体、不 subset。
//! 这一点在报告的 `scope` 里也写出来，因为"扫描通过"很容易被误读成"渲染一定复现"——
//! 我们按名字读表匹配，没有问 libass/DirectWrite 的 fallback 链（ISSUES AG1）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};
use font_service::db::{CacheState, FontDb, FONT_EXTS};
use font_service::report::Verdict;

use crate::args::{Command, Invocation};
use crate::output::{Envelope, WriteRecord};
use crate::pipeline;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

/// 默认字体目录：系统目录 + 当前用户的注册字体目录。
///
/// 只扫 `C:\Windows\Fonts` 是 17 §2 明确禁止的做法——从 Win10 1703 起，
/// 用户在 设置→字体 里装的字体落在 `%LOCALAPPDATA%\Microsoft\Windows\Fonts`，
/// 系统目录里根本没有那个文件。
pub(crate) fn default_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["WINDIR", "SYSTEMROOT"] {
        if let Ok(w) = std::env::var(var) {
            let p = Path::new(&w).join("Fonts");
            if p.is_dir() {
                dirs.push(p);
                break;
            }
        }
    }
    for var in ["LOCALAPPDATA", "APPDATA"] {
        if let Ok(a) = std::env::var(var) {
            let p = Path::new(&a)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts");
            if p.is_dir() {
                dirs.push(p);
                break;
            }
        }
    }
    dedupe(dirs)
}

fn dedupe(mut dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs.sort();
    dirs
}

pub fn run_font(inv: &Invocation) -> Result<Envelope> {
    let cmd = inv.command.name();
    let Command::FontScan {
        input,
        fonts_dirs,
        registry,
        copy_fonts,
    } = &inv.command
    else {
        return Err(cli_err(180, "font 只实现了 scan"));
    };
    let loaded = pipeline::load_text(input)?;
    // 直接用 ASS 文本而不是 AST：扫描器关心的是 Format 行的字段次序与 override 时序
    let requests = font_service::usage::scan_text(&loaded.text)?;

    let dirs = if fonts_dirs.is_empty() {
        default_font_dirs()
    } else {
        dedupe(fonts_dirs.clone())
    };
    if dirs.is_empty() {
        return Err(cli_err(181, "找不到任何字体目录，无法给出字体可用性结论")
            .with_action("用 --fonts-dir DIR 指明字体库位置（可重复）"));
    }
    let missing_dir: Vec<String> = dirs
        .iter()
        .filter(|d| !d.is_dir())
        .map(|d| d.display().to_string())
        .collect();
    if !missing_dir.is_empty() {
        // 指错了目录就不能当作"这个库里没有"：那会把拼写错误报成缺字体
        return Err(cli_err(182, "指定的字体目录不存在").with_detail(missing_dir.join("；")));
    }

    let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
    // 注册表登记的字体（17 §2 的"字体注册"）。关掉它只会让库变小，不会让结论变宽松：
    // Missing 的含义始终是"在我扫过的这些来源里按名字找不到"，来源本身在报告里列清楚。
    let reg = if *registry {
        font_service::registry::enumerate()
    } else {
        font_service::registry::RegistryReport::default()
    };
    // 带磁盘缓存：`font scan` 是"问这台机器有什么字体"的命令，会被反复调用，而字体在两次
    // 调用之间通常没变。命中时不重读没被用到的文件，但凡进了结论的 face 都在本次被重读并
    // 复核内容哈希；复核说不一致就退回全量重扫一次（`evaluate_or_rescan`），所以缓存最多
    // 让我们慢一次，不会让我们答错（依据与实测数字都在 font_service::cache 头部，ISSUES AI5）。
    // 两个阶段各自计时：这条命令慢的时候，报告必须能指出是"建库"慢还是"评级"慢。
    // 第一版没有这两个数，我只能拿整条命令的墙钟时间猜，而猜错过一次（AI5 的测量）。
    // 注意：缓存被推翻时重扫的耗时记在 evaluate_ms 里，报告用 cache=stale-refreshed 说明这件事。
    let mode = crate::fontenv::cache_mode(inv);
    let t0 = std::time::Instant::now();
    let mut db = FontDb::scan_cached(&refs, &reg.font_files(), mode);
    let build_ms = t0.elapsed().as_millis() as u64;
    let t1 = std::time::Instant::now();
    let matches = font_service::evaluate_or_rescan(&mut db, &refs, &reg.font_files(), &requests)?;
    let evaluate_ms = t1.elapsed().as_millis() as u64;

    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for m in &matches {
        *counts.entry(verdict_key(&m.verdict)).or_insert(0) += 1;
    }
    let glyph_gaps: usize = matches
        .iter()
        .filter(|m| !m.missing_glyphs.is_empty())
        .count();
    let mut warnings: Vec<String> = Vec::new();
    if db.cache == CacheState::Refreshed {
        // 缓存被使用点复核推翻：多付的那次全量重扫算在 evaluate_ms 里，不说明的话
        // 读者看到的是"评级突然变慢了"。
        warnings.push(
            "磁盘缓存的索引与文件现状不符（同长度且同 mtime 的原地覆盖），本次已退回全量重扫，新索引已写回"
                .to_string(),
        );
    }
    if db.faces.is_empty() {
        warnings.push("字体库里一条 face 都没读到：上面的 Missing 全部不可信".to_string());
    }
    // 逐条 note 会变成上百行噪声（本机 C:\Windows\Fonts 就有 192 个 .fon），
    // 但"归并"不等于"丢掉"：原因、数量、样本都要留得下来查。
    let mut grouped: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
    for n in &db.notes {
        let e = grouped.entry(n.reason.as_str()).or_insert((0, Vec::new()));
        e.0 += 1;
        if e.1.len() < 3 {
            e.1.push(n.path.clone());
        }
    }
    for (reason, (count, samples)) in &grouped {
        warnings.push(format!(
            "{count} 个文件没进数据库：{reason}（例：{}）",
            samples.join("、")
        ));
    }
    // 后缀本来就是 ttf/otf/ttc/otc 却没读成：那不是"预期跳过"，那是真的丢字体
    let lost: Vec<&str> = db
        .notes
        .iter()
        .filter(|n| {
            Path::new(&n.path)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| FONT_EXTS.contains(&s.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|n| n.path.as_str())
        .collect();
    if !lost.is_empty() {
        warnings.push(format!(
            "{} 个带字体后缀的文件解析失败，必须单独处理（前若干个：{}）",
            lost.len(),
            lost.iter().take(5).copied().collect::<Vec<_>>().join("、")
        ));
    }
    if *registry {
        if reg.unsupported {
            warnings.push("本平台没有注册表这套机制：只用了目录扫描".to_string());
        }
        for (hive, why) in &reg.open_errors {
            warnings.push(format!("注册表 {hive} 读不了：{why}"));
        }
        for (hive, why) in &reg.field_errors {
            warnings.push(format!("注册表 {hive} 有条目读不了：{why}"));
        }
        let unresolved = reg.entries.iter().filter(|e| e.path.is_none()).count();
        if unresolved > 0 {
            let samples: Vec<String> = reg
                .entries
                .iter()
                .filter(|e| e.path.is_none())
                .take(3)
                .map(|e| format!("{}：{}", e.display_name, e.note.clone().unwrap_or_default()))
                .collect();
            warnings.push(format!(
                "{unresolved} 条字体登记解析不到文件（例：{}）",
                samples.join("、")
            ));
        }
    }
    if glyph_gaps > 0 {
        warnings.push(format!(
            "{glyph_gaps} 个 face 有缺字。缺字清单按 cmap 判定，不含 OpenType shaping 层面的替换（见 scope）"
        ));
    }

    // 同一份算法也用于渲染缓存 key（17 §7）：把身份放进报告，"scan 说的环境"与
    // "render 用的环境"才是同一个东西，而不是两套各算各的。
    let embedded = font_service::parse_font_attachments(&loaded.text)?;
    let identity = font_service::identify(&db, &matches, &embedded);
    let report = serde_json::json!({
        "render_identity": {
            "digest": identity.digest,
            "policy": identity.policy,
            "faces_used": identity.faces_used,
            "faces_missing": identity.faces_missing,
            "embedded_fonts": identity.attachments,
            "cache": identity.cache.label(),
        },
        "input": input.display().to_string(),
        "fonts_dirs": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
        "database": {
            "faces": db.faces.len(),
            "files_read": db.files_read,
            "unreadable_or_unsupported": db.notes.len(),
            "cache": db.cache.label(),
            "cache_meaning": db.cache.meaning(),
            "build_ms": build_ms,
            "evaluate_ms": evaluate_ms,
        },
        "requests": requests.len(),
        "verdict_states": Verdict::SPEC_STATES.iter().map(verdict_key).collect::<Vec<_>>(),
        "verdict_counts": counts,
        "faces_with_missing_glyphs": glyph_gaps,
        "registry": {
            "read": *registry,
            "unsupported_platform": reg.unsupported,
            "entries": reg.entries.len(),
            "resolved": reg.entries.iter().filter(|e| e.path.is_some()).count(),
            "files_only_from_registry": db.registry_only,
            "open_errors": reg.open_errors.len(),
            "field_errors": reg.field_errors.len(),
        },
        "matches": serde_json::to_value(&matches).map_err(|e| {
            cli_err(183, "字体报告序列化失败").with_detail(e.to_string())
        })?,
        "scope": [
            "默认 report only：除了 --out 的报告不写产物；给了 --copy-fonts 才按 17 §6 第二档复制整份字体并写 manifest。两种模式都不 subset（AG2）",
            "匹配读的是字体文件里的 name/OS2/cmap 表，未使用 DirectWrite 权威（17 §2，ISSUES AG1）",
            "未询问 libass/VSFilterMod 的 fallback 链：Missing 表示按名字找不到，不等于渲染必然失败",
            "缺字按 cmap 码位判定；ligature/GSUB 层面的覆盖未建模（17 §5 的 corpus 未建，ISSUES AG2）",
        ],
        "warnings": warnings,
    });
    let mut data = report;
    let mut writes: Vec<WriteRecord> = Vec::new();

    // 17 §6 的两档：默认只出报告；给了 --copy-fonts 就再走"复制整份 + manifest"
    if let Some(dest) = copy_fonts.as_deref() {
        let (copies, manifest) = crate::deliver::copy_fonts(inv, &matches, dest)?;
        writes = copies;
        data["delivery"] = serde_json::json!({
            "mode": manifest["mode"],
            "delivered_dir": manifest["delivered_dir"],
            "distinct_files": manifest["distinct_files"],
            "manifest": manifest,
        });
    } else {
        data["delivery"] = serde_json::json!({
            "mode": "report only",
            "note": "没有 --copy-fonts：除 --out 指定的报告外不写任何产物（17 §6 第一档）",
        });
    }

    if let Some(path) = inv.out.as_deref() {
        let pretty = serde_json::to_vec_pretty(&data)
            .map_err(|e| cli_err(184, "字体报告序列化失败").with_detail(e.to_string()))?;
        let emitted = pipeline::emit(
            Some(path),
            path,
            &pretty,
            |b| {
                serde_json::from_slice::<serde_json::Value>(b)
                    .map(|_| ())
                    .map_err(|e| cli_err(185, "字体报告不是合法 JSON").with_detail(e.to_string()))
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
    }
    let mut env = Envelope::ok(cmd, data);
    env.writes = writes;
    Ok(env)
}

/// 计数用的键直接取 `Verdict` 的序列化名，不再手写第二份映射：
/// 手抄的清单会和报告里的 verdict 拼法漂移（doctor 的未实现清单栽过同一个跟头，ISSUES AC4）。
fn verdict_key(v: &Verdict) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use task_engine::OverwritePolicy;

    fn inv(input: &Path, dirs: Vec<PathBuf>, out: Option<PathBuf>) -> Invocation {
        Invocation {
            command: Command::FontScan {
                input: input.to_path_buf(),
                fonts_dirs: dirs,
                registry: false,
                copy_fonts: None,
            },
            json: true,
            out,
            dry_run: false,
            overwrite: OverwritePolicy::ReplaceAfterVerify,
            inject_fonts: false,
            no_font_cache: false,
        }
    }

    const ASS: &str = "[V4+ Styles]\r\nFormat: Name, Fontname\r\nStyle: D,Arial\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,D,你好\r\n";

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_dirs_are_deduped_and_sorted() {
        let a = PathBuf::from("C:\\WINDOWS\\Fonts");
        let dirs = dedupe(vec![a.clone(), PathBuf::from("C:\\other"), a.clone()]);
        assert_eq!(dirs.len(), 2, "同一目录不能扫两遍：{dirs:?}");
        assert!(dirs.windows(2).all(|w| w[0] < w[1]), "必须有序：{dirs:?}");
    }

    /// 指错目录必须是错误，不能悄悄当成"这个库里没有"——那等于把拼写错误报成缺字体。
    #[test]
    fn a_nonexistent_fonts_dir_is_an_error_not_a_missing_font() {
        let dir = scratch("font-nodir").join("typo");
        let ass = scratch("font-nodir").join("x.ass");
        std::fs::write(&ass, ASS).unwrap();
        let i = inv(&ass, vec![dir.clone()], None);
        let e = run_font(&i).unwrap_err();
        assert_eq!(e.code_string(), "STK-INP-0182", "{e:?}");
        assert!(e.title.contains("字体目录"), "{e}");
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    /// 端到端：真的建库、真的评级、真的写报告。字体库里放一个从系统复制来的 TTF，
    /// 请求的却是库里没有的家族，报告必须同时给出 missing 与缺字。
    #[test]
    fn scan_reports_the_database_and_writes_the_json() {
        let dir = scratch("font-scan");
        let fonts = dir.join("fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        let mut src: Option<PathBuf> = None;
        if let Ok(w) = std::env::var("WINDIR") {
            let sys = Path::new(&w).join("Fonts");
            if let Ok(rd) = std::fs::read_dir(&sys) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("ttf") {
                        src = Some(p);
                        break;
                    }
                }
            }
        }
        let Some(src) = src else {
            panic!("本机 %WINDIR%\\Fonts 里没有任何 .ttf，无法构造真实字体库");
        };
        std::fs::copy(&src, fonts.join(src.file_name().unwrap())).unwrap();
        let ass = dir.join("x.ass");
        std::fs::write(&ass, ASS).unwrap();
        let report = dir.join("report.json");

        let i = inv(&ass, vec![fonts.clone()], Some(report.clone()));
        let env = run_font(&i).unwrap();
        assert!(env.ok, "{env:?}");
        let d = env.data.clone().unwrap();
        assert_eq!(d["database"]["faces"].as_u64().unwrap(), 1);
        assert_eq!(d["requests"].as_u64().unwrap(), 1);
        let m = &d["matches"][0];
        assert_eq!(m["request"].as_str().unwrap(), "Arial");
        assert!(
            m["verdict"].as_str().unwrap() != "exact",
            "库里只有随机一个字体，不该判成 exact：{m}"
        );
        assert!(
            !m["missing_glyphs"].as_str().unwrap().is_empty() || m["face"].is_null(),
            "要么给出缺字清单，要么明说找不到 face：{m}"
        );
        assert!(d["scope"].as_array().unwrap().len() >= 4, "边界必须写全");
        assert_eq!(env.writes.len(), 1, "{env:?}");
        let written = std::fs::read(&report).unwrap();
        assert_eq!(written.len(), env.writes[0].bytes as usize);
        let back: serde_json::Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(back["requests"], d["requests"], "写盘内容必须与信封一致");

        // dry-run 不许碰盘：报告路径不存在时也不能被创建
        std::fs::remove_file(&report).unwrap();
        let mut dry = inv(&ass, vec![fonts.clone()], Some(report.clone()));
        dry.dry_run = true;
        let env = run_font(&dry).unwrap();
        assert_eq!(env.writes[0].state, "would-write", "{env:?}");
        assert!(!report.exists(), "dry-run 写了盘");
        std::fs::remove_dir_all(&dir).ok();
    }
}
