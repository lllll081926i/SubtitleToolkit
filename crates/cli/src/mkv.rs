//! `mkv identify|extract|remux`（18_CONTAINER_HELPER）。
//!
//! 这一层只做三件事：把 CLI 参数翻译成 adapter 调用、把 adapter 的结果整理成报告、
//! 把产物登记进 `writes`。容器语义全在 `container-adapter` 里，这里不解析任何 EBML。
//!
//! 18 §4 的"绝不原地修改"在本文件有第二道防线：adapter 已经拒绝"输出 == 源"，
//! 这里再核对一次 `--out` 与输入的规范化路径，因为 remux 是唯一会写用户目录的命令。

use std::path::{Path, PathBuf};

use container_adapter::{self as mkv, Tools};
use core_model::error::{Domain, Result, StkError};

use crate::args::{Command, Invocation, MkvAction};
use crate::output::{Envelope, WriteRecord};

// 本文件的错误码占用 Input 域的 250–260 段。
// 这一段是空的（Input 域在 190–200 已被 deliver.rs 与其它 crate 用满，
// 实测 200 同时在 codec-srt 里表达"偏移量超出可处理范围"）。
// 撞码会被 `core-model/tests/error_code_uniqueness.rs` 拦住，
// 那个扫描器是"这个域里哪些数已经用了"的唯一权威答案。

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

/// 找 MKVToolNix。优先级：`--tools-dir` → `STK_MKVTOOLNIX_DIR` → `STK_MKVMERGE`/`STK_MKVEXTRACT`
/// → PATH。绝不猜硬盘上的位置：指向错误的 exe 会以"非零退出"的形式出现，
/// 那比"未找到"更难查。
fn resolve_tools(tools_dir: Option<&Path>) -> Result<Tools> {
    let (merge, extract) = if let Some(dir) = tools_dir {
        if !dir.is_dir() {
            return Err(cli_err(250, "--tools-dir 不是目录").with_detail(dir.display().to_string()));
        }
        (dir.join("mkvmerge.exe"), dir.join("mkvextract.exe"))
    } else if let Ok(dir) = std::env::var("STK_MKVTOOLNIX_DIR") {
        let d = PathBuf::from(dir);
        (d.join("mkvmerge.exe"), d.join("mkvextract.exe"))
    } else if let (Ok(m), Ok(e)) = (
        std::env::var("STK_MKVMERGE"),
        std::env::var("STK_MKVEXTRACT"),
    ) {
        (PathBuf::from(m), PathBuf::from(e))
    } else {
        (
            PathBuf::from("mkvmerge.exe"),
            PathBuf::from("mkvextract.exe"),
        )
    };
    for exe in [&merge, &extract] {
        if exe.is_absolute() && !exe.is_file() {
            return Err(cli_err(251, "找不到 MKVToolNix 组件")
                .with_detail(exe.display().to_string())
                .with_action(
                    "用 --tools-dir DIR 指向安装目录，或设 STK_MKVTOOLNIX_DIR / STK_MKVMERGE+STK_MKVEXTRACT",
                ));
        }
    }
    Ok(Tools::new(merge, extract))
}

pub fn run_mkv(inv: &Invocation) -> Result<Envelope> {
    let cmd = inv.command.name();
    let Command::Mkv { action, tools_dir } = &inv.command else {
        return Err(cli_err(252, "mkv 命令分派错误"));
    };
    let tools = resolve_tools(tools_dir.as_deref())?;
    // 工具身份进报告：21 §9 要求能说清"是哪个二进制干的活"
    let identity = tools.identity().ok();

    match action {
        MkvAction::Identify { input } => {
            let id = mkv::identify(&tools, input)?;
            let data = serde_json::json!({
                "input": input.display().to_string(),
                "container": {
                    "recognized": id.container.recognized,
                    "supported": id.container.supported,
                    "type": id.container.kind,
                },
                "file_name": id.file_name,
                "tracks": id.tracks.iter().map(|t| serde_json::json!({
                    "id": t.id,
                    "type": t.kind,
                    "codec": t.codec,
                    "language": t.properties.language,
                    "language_ietf": t.properties.language_ietf,
                    "track_name": t.properties.track_name,
                    "default": t.properties.default_track,
                    "forced": t.properties.forced_track,
                })).collect::<Vec<_>>(),
                "attachments": id.attachments.iter().map(|a| serde_json::json!({
                    "id": a.id,
                    "file_name": a.file_name,
                    "description": a.description,
                    "content_type": a.content_type,
                    "size": a.size,
                })).collect::<Vec<_>>(),
                "subtitle_tracks": id.subtitle_tracks().iter().map(|t| t.id).collect::<Vec<_>>(),
                "font_attachments": id.font_attachments().iter().map(|a| a.file_name.clone()).collect::<Vec<_>>(),
                "chapters": id.chapter_count(),
                "tools": identity.as_ref().map(|i| serde_json::json!({
                    "mkvmerge": i.mkvmerge.path,
                    "mkvmerge_sha256": i.mkvmerge.digest_sha256,
                    "mkvextract": i.mkvextract.path,
                    "mkvextract_sha256": i.mkvextract.digest_sha256,
                })),
                // mkvmerge 自己报的问题：它说"某轨解码器缺失"这类事时，识别仍然成功，
                // 但下游一定会撞上。吞掉它就等于把上游的告警变成我们的意外。
                "mkvmerge_errors": id.errors,
                "mkvmerge_warnings": id.warnings,
                // 18 §1：不做转码、不做章节编辑 —— 这些能力边界要在报告里写出来
                "scope": [
                    "只调用 MKVToolNix：本工具不解析也不重写 Matroska",
                    "不做视频/音频转码，不做章节编辑（18 §3）",
                ],
            });
            finish(inv, cmd, data, Vec::new())
        }

        MkvAction::Extract {
            input,
            tracks,
            attachments,
        } => {
            // 先确认轨道/附件 id 真存在于源文件：拿一个不存在的 id 去跑 mkvextract，
            // 错误信息是它自己的退出码，用户看不出是"我写错了 id"还是"文件有问题"。
            let id = mkv::identify(&tools, input)?;
            for (track_id, out) in tracks {
                if id.track(*track_id).is_none() {
                    return Err(
                        cli_err(253, &format!("源文件里没有轨道 {track_id}")).with_detail(format!(
                            "可用轨道：{:?}",
                            id.tracks.iter().map(|t| t.id).collect::<Vec<_>>()
                        )),
                    );
                }
                mkv::extract_track(&tools, input, *track_id, out)?;
            }
            for (att_id, out) in attachments {
                if !id.attachments.iter().any(|a| a.id == *att_id) {
                    return Err(
                        cli_err(254, &format!("源文件里没有附件 {att_id}")).with_detail(format!(
                            "可用附件：{:?}",
                            id.attachments.iter().map(|a| a.id).collect::<Vec<_>>()
                        )),
                    );
                }
                mkv::extract_attachment(&tools, input, *att_id, out)?;
            }
            let writes: Vec<WriteRecord> = tracks
                .iter()
                .map(|(_, p)| p)
                .chain(attachments.iter().map(|(_, p)| p))
                .map(|p| WriteRecord {
                    path: p.display().to_string(),
                    bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                    state: "extracted".to_string(),
                })
                .collect();
            let data = serde_json::json!({
                "input": input.display().to_string(),
                "extracted_tracks": tracks.iter().map(|(i, p)| serde_json::json!({
                    "id": i, "out": p.display().to_string(),
                })).collect::<Vec<_>>(),
                "extracted_attachments": attachments.iter().map(|(i, p)| serde_json::json!({
                    "id": i, "out": p.display().to_string(),
                })).collect::<Vec<_>>(),
                "source_untouched": true,
                "tools": identity.as_ref().map(|i| serde_json::json!({
                    "mkvmerge": i.mkvmerge.path,
                    "mkvextract": i.mkvextract.path,
                })),
            });
            finish(inv, cmd, data, writes)
        }

        MkvAction::Remux {
            input,
            subtitle_tracks,
            all_subtitles,
            keep_video,
            keep_audio,
            attach_fonts,
            track_name,
            track_language,
            default_track,
            forced_track,
        } => {
            // 18 §4 的唯一写盘路径要求显式输出：没有 --out 就无从谈起"不原地修改"。
            let out = inv.out.as_deref().ok_or_else(|| {
                cli_err(260, "mkv remux 需要 --out NEW.mkv（18 §4 绝不原地修改）")
                    .with_action("例：mkv remux in.mkv --out out.mkv")
            })?;
            // 18 §4：输出不得等于输入。adapter 里还有一道，这里早失败一步、错误更贴近参数。
            guard_not_in_place(input, out)?;

            let src = mkv::identify(&tools, input)?;
            let video: Vec<u64> = if *keep_video {
                src.tracks
                    .iter()
                    .filter(|t| t.kind == "video")
                    .map(|t| t.id)
                    .collect()
            } else {
                Vec::new()
            };
            let audio: Vec<u64> = if *keep_audio {
                src.tracks
                    .iter()
                    .filter(|t| t.kind == "audio")
                    .map(|t| t.id)
                    .collect()
            } else {
                Vec::new()
            };
            let subs: Vec<u64> = if *all_subtitles {
                src.subtitle_tracks().iter().map(|t| t.id).collect()
            } else {
                // 显式给 id 时先验证：写错一个 id 会让 mkvmerge 保留零条字幕，
                // 而报告看起来像成功了（元数据核对会发现，但那时已经在产物上）
                for id in subtitle_tracks {
                    if src.track(*id).is_none() {
                        return Err(cli_err(255, &format!("源文件里没有轨道 {id}")).with_detail(
                            format!(
                                "字幕轨：{:?}",
                                src.subtitle_tracks()
                                    .iter()
                                    .map(|t| t.id)
                                    .collect::<Vec<_>>()
                            ),
                        ));
                    }
                    if src.track(*id).map(|t| t.kind.as_str()) != Some("subtitles") {
                        return Err(cli_err(256, &format!("轨道 {id} 不是字幕轨"))
                            .with_action("--subtitle-tracks 只接受字幕轨 id"));
                    }
                }
                subtitle_tracks.clone()
            };

            // 18 §2 的 language/name/default/forced：用 mkvmerge 的 -M 语法施加到目标轨。
            // 每个选项名与它的取值都是**两个 argv 元素**（同 push_track_selection 的理由）。
            // --default-track/--forced-track 必须显式写 yes 或 no：
            // mkvmerge 里这两个选项不给值时是 no，静默把轨降级不是我们想干的。
            let mut extra: Vec<String> = Vec::new();
            for (id, name) in track_name {
                extra.push("--track-name".to_string());
                extra.push(format!("{id}:{name}"));
            }
            for (id, lang) in track_language {
                extra.push("--language".to_string());
                extra.push(format!("{id}:{lang}"));
            }
            for id in default_track {
                extra.push("--default-track".to_string());
                extra.push(format!("{id}:yes"));
            }
            for id in forced_track {
                extra.push("--forced-track".to_string());
                extra.push(format!("{id}:yes"));
            }

            let outcome = mkv::remux_with(
                &tools,
                input,
                out,
                &subs,
                &video,
                &audio,
                attach_fonts,
                &extra,
            )?;
            let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            let data = serde_json::json!({
                "input": input.display().to_string(),
                "output": out.display().to_string(),
                "source_untouched": true,
                "source_note": "18 §4：源文件只被读取；产物是 --out 指定的新文件",
                "source_sha256": file_sha256(input).ok(),
                "output_sha256": file_sha256(out).ok(),
                "kept": {
                    "video_tracks": outcome.kept_video,
                    "audio_tracks": outcome.kept_audio,
                    "subtitle_tracks": outcome.kept_subtitles,
                },
                "dropped": {
                    "video_tracks": outcome.dropped_video,
                    "audio_tracks": outcome.dropped_audio,
                    "subtitle_tracks": outcome.dropped_subtitles,
                },
                "attached_fonts": attach_fonts.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "attached_fonts_applied": outcome.attached_fonts,
                "attached_fonts_skipped": outcome.skipped_fonts,
                "attach_policy": "源文件里已有同名同大小附件时跳过，不重复附加（依名字+字节数判定）",
                "before": {
                    "tracks": outcome.before.tracks.len(),
                    "attachments": outcome.before.attachments.len(),
                    "chapters": outcome.before.chapter_count(),
                },
                "after": {
                    "tracks": outcome.after.tracks.len(),
                    "attachments": outcome.after.attachments.len(),
                    "chapters": outcome.after.chapter_count(),
                },
                "metadata_preserved": true,
                "tools": identity.as_ref().map(|i| serde_json::json!({
                    "mkvmerge": i.mkvmerge.path,
                    "mkvmerge_sha256": i.mkvmerge.digest_sha256,
                    "mkvextract": i.mkvextract.path,
                    "mkvextract_sha256": i.mkvextract.digest_sha256,
                })),
                "scope": [
                    "只重封装：视频/音频码流逐字节复制，不做转码（18 §3）",
                    "产物写完会重新 identify 并逐轨核对元数据（18 §4）",
                ],
            });
            let writes = vec![WriteRecord {
                path: out.display().to_string(),
                bytes,
                state: if inv.dry_run {
                    "dry-run".to_string()
                } else {
                    "written".to_string()
                },
            }];
            finish(inv, cmd, data, writes)
        }
    }
}

/// 18 §4 的第二道防线：输出与输入的规范化路径相同就拒绝。
fn guard_not_in_place(input: &Path, out: &Path) -> Result<()> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (a, b) = (canon(input), canon(out));
    if a == b {
        return Err(cli_err(
            257,
            "输出路径与输入是同一个文件：18 §4 要求绝不原地修改 MKV",
        )
        .with_detail(out.display().to_string())
        .with_action("把 --out 改成一个新的文件名"));
    }
    Ok(())
}

fn file_sha256(p: &Path) -> Result<String> {
    let bytes = std::fs::read(p).map_err(StkError::from)?;
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn finish(
    inv: &Invocation,
    cmd: &'static str,
    mut data: serde_json::Value,
    mut writes: Vec<WriteRecord>,
) -> Result<Envelope> {
    if let Some(path) = inv.out.as_deref() {
        let pretty = serde_json::to_vec_pretty(&data)
            .map_err(|e| cli_err(258, "mkv 报告序列化失败").with_detail(e.to_string()))?;
        let emitted = crate::pipeline::emit(
            Some(path),
            path,
            &pretty,
            |b| {
                serde_json::from_slice::<serde_json::Value>(b)
                    .map(|_| ())
                    .map_err(|e| cli_err(259, "mkv 报告不是合法 JSON").with_detail(e.to_string()))
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
    let _ = &mut data;
    let mut env = Envelope::ok(cmd, data);
    env.writes = writes;
    Ok(env)
}
