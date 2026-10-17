//! 参考视频探测：GUI 的“拖入视频拿建议”那一条（16 §2、AT-PROBE-001）。
//!
//! ## 为什么必须有这个模块
//!
//! 这个位置原本是一句写死的字符串：`建议帧率 23.976 | 分辨率 3840x2160`。
//! 它对**任何**视频都“对”，所以也没有任何用户反馈能暴露它其实是假的
//! —— 这是伪装成功能的常量。
//!
//! ## 只建议，不改
//!
//! 16 §2 与 AT-PROBE-001 都写明：探测结果只是**建议**，不得自动应用变速。
//! 所以这里返回的是给人看的文案 + 一个待用户确认的提议；
//! 真正的应用发生在 `Message::ApplyVideoAssist`（用户点了才会走）。
//!
//! ## 探不到时怎么办
//!
//! 不给数字。`banner` 会说清楚为什么探不到（没装 ffprobe / 读不了 / 不是视频），
//! 并给出下一步——让用户去猜“为什么没有建议”比不给建议难查得多。

use std::path::Path;

use crate::state::inputs::VideoAssist;

pub struct ProbeOutcome {
    pub assist: VideoAssist,
    /// 给横幅显示的一句话（包含“探到了什么”或“为什么没探到”）
    pub banner: String,
}

/// 探测一个参考视频。永不 panic、永不返回 Err：探测失败是**要给用户看的文案**，
/// 不是可以把整个界面卡住的错误。
pub fn probe_video(path: &Path) -> ProbeOutcome {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let ffprobe = match cli::run::find_ffprobe() {
        Some(p) => p,
        None => {
            return ProbeOutcome {
                assist: VideoAssist {
                    path: path.to_path_buf(),
                    fps: None,
                    resolution: None,
                    is_hdr: false,
                    summary: "未探测（找不到 ffprobe）".to_string(),
                },
                banner: format!(
                    "参考视频 {name} 已载入，但找不到 ffprobe，无法给出帧率/分辨率建议。\
                     设 STK_FFPROBE 指向 ffprobe.exe 后重试；也可以手动在右侧指定画布。"
                ),
            };
        }
    };

    let runner = media_probe::SystemProbe::new(&ffprobe, std::time::Duration::from_secs(30));
    let json = match media_probe::ProbeRunner::probe(&runner, path) {
        Ok(j) => j,
        Err(e) => {
            return ProbeOutcome {
                assist: VideoAssist {
                    path: path.to_path_buf(),
                    fps: None,
                    resolution: None,
                    is_hdr: false,
                    summary: format!("探测失败：{}", e.code_string()),
                },
                banner: format!(
                    "参考视频 {name} 探测失败（{}：{}）——建议不会自动应用。",
                    e.code_string(),
                    e.title
                ),
            };
        }
    };

    // 复用 media-probe 的建议逻辑，不在 GUI 里另写一套阈值。
    // 两套阈值意味着“同样的视频，GUI 与 CLI 给不同建议”。
    let streams = match media_probe::parse_video_streams(&json) {
        Ok(s) => s,
        Err(e) => {
            return ProbeOutcome {
                assist: VideoAssist {
                    path: path.to_path_buf(),
                    fps: None,
                    resolution: None,
                    is_hdr: false,
                    summary: "探测输出无法解析".to_string(),
                },
                banner: format!(
                    "参考视频 {name} 的 ffprobe 输出无法解析（{}）：{}",
                    e.code_string(),
                    e.title
                ),
            };
        }
    };

    // 复用 media-probe 的建议逻辑，不在 GUI 里另写一套阈值。
    // 两套阈值意味着“同样的视频，GUI 与 CLI 给不同建议”。
    // 取第一条视频流：横幅只需说清一件最相关的事，多流的细致对比属于 CLI 的职责。
    let Some(stream) = streams.first() else {
        return ProbeOutcome {
            assist: VideoAssist {
                path: path.to_path_buf(),
                fps: None,
                resolution: None,
                is_hdr: false,
                summary: "文件里没有视频流".to_string(),
            },
            banner: format!("参考视频 {name} 里没有视频流，无法给出建议。"),
        };
    };
    let set = media_probe::suggest_for(stream, &media_probe::SubtitleContext::default());
    // 帧率取首选候选：`fps_candidates()` 已按置信度排序。
    // 分辨率直接用视频流的真实尺寸（这是 ffprobe 的事实，不是“建议”）；
    // 画布该不该被改写是另一回事，那由用户点“应用”时决定，
    // 并受 AT-CANVAS-002 约束（2.39:1 不得被自动改成非 16:9）。
    let fps = set.fps_candidates().first().copied();
    let resolution = Some((stream.width, stream.height));

    let mut bits: Vec<String> = Vec::new();
    match fps {
        Some(f) => bits.push(format!("建议帧率 {}", f.to_label())),
        None => bits.push("未能确定帧率".to_string()),
    }
    match resolution {
        Some((w, h)) => bits.push(format!("分辨率 {w}x{h}")),
        None => bits.push("未能确定分辨率".to_string()),
    }
    let banner = format!(
        "参考视频 {name}：{}。这些只是建议，不会自动应用——确认后点“应用”。",
        bits.join(" | ")
    );

    ProbeOutcome {
        assist: VideoAssist {
            path: path.to_path_buf(),
            // `as_rational()` 是公开的取值通路（`Fps` 的字段是私有的）。
            fps: fps.map(|f| f.as_rational()),
            resolution,
            is_hdr: stream.hdr_signaled,
            summary: bits.join(" | "),
        },
        banner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探不到 ffprobe 或不是视频时，必须**说清楚**，绝不编一个数字。
    #[test]
    fn a_non_video_input_produces_an_explanation_not_a_fake_number() {
        let p = std::env::temp_dir().join(format!("stk-gui-probe-{}.txt", std::process::id()));
        std::fs::write(&p, b"not a video").unwrap();
        let out = probe_video(&p);
        assert!(out.assist.fps.is_none(), "不能给出一个编出来的帧率");
        assert!(out.assist.resolution.is_none());
        assert!(!out.banner.is_empty(), "必须说明为什么");
        assert!(
            out.banner.contains("stk-gui-probe"),
            "文案要带上文件名，好定位是哪个输入：{}",
            out.banner
        );
        // 不能出现“看起来像建议”的硬编码数字
        assert!(
            !out.banner.contains("23.976") && !out.banner.contains("3840x2160"),
            "不得回到写死建议的老路：{}",
            out.banner
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_summary_always_reflects_what_was_actually_found() {
        let p = std::env::temp_dir().join("definitely-not-here.mkv");
        let out = probe_video(&p);
        // 没有 ffprobe 或文件不存在：summary 必须写“未探测”或“失败”，不能是空的
        assert!(!out.assist.summary.is_empty(), "{:?}", out.assist);
    }
}
