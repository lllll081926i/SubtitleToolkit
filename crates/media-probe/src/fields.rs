//! ffprobe `-show_streams -show_format -print_format json` 的字段读取。
//! 只解析本项目用到的字段；未知字段忽略，字段缺失用 Option 表达（16 §5：探测残缺不得阻塞主流程）。

use core_model::error::{Domain, Result, StkError};
use core_model::time::{Fps, Rational};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ColorMeta {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub matrix: Option<String>,
    pub range: Option<String>,
    pub pix_fmt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoStream {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub r_frame_rate: Option<Rational>,
    pub avg_frame_rate: Option<Rational>,
    /// 秒，有理数（ffprobe 的 duration 是十进制字符串）
    pub duration: Option<Rational>,
    pub color: ColorMeta,
    /// 是否出现 HDR10/PQ 相关的 side data 或 transfer 特征
    pub hdr_signaled: bool,
}

impl VideoStream {
    pub fn fps_candidates(&self) -> Vec<Fps> {
        let mut out = Vec::new();
        for r in [self.r_frame_rate, self.avg_frame_rate]
            .into_iter()
            .flatten()
        {
            if let Ok(f) = Fps::new(r) {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        out
    }

    pub fn is_hdr(&self) -> bool {
        self.hdr_signaled
            || self.color.transfer.as_deref() == Some("smpte2084")
            || self.color.transfer.as_deref() == Some("arib-std-b67")
    }
}

/// 解析 ffprobe JSON 里的全部视频流。
pub fn parse_video_streams(json: &str) -> Result<Vec<VideoStream>> {
    let root: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        StkError::simple(Domain::Dependency, 10, "ffprobe 输出不是合法 JSON")
            .with_detail(e.to_string())
    })?;
    let streams = root
        .get("streams")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StkError::simple(Domain::Dependency, 11, "ffprobe JSON 缺 streams 数组"))?;
    let mut out = Vec::new();
    for s in streams {
        if s.get("codec_type").and_then(|v| v.as_str()) != Some("video") {
            continue;
        }
        let index = s.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let width = s.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let height = s.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if width == 0 || height == 0 {
            // 有些容器给出 0 尺寸流，跳过而不是让整次探测失败
            continue;
        }
        // **真实 ffprobe 的字段是扁平的**：`color_transfer` / `color_primaries` /
        // `color_space` / `color_range`（本机 gyan 构建实测，见 tests/fixtures/*.ffprobe.json）。
        // 早先这里只认嵌套的 `color_metadata:{transfer:…}`，那是我按想象中的 schema 写的 fixture
        // 造出来的形状 —— 单测全绿，真二进制却读不出任何色彩信息（ISSUES AF1）。
        // 嵌套形状仍然兜底：不同构建/封装的导出确实出现过嵌套形式。
        let str_at = |keys: &[&str]| -> Option<String> {
            keys.iter()
                .filter_map(|k| s.get(*k).and_then(|v| v.as_str()))
                .next()
                .map(str::to_string)
        };
        let cmeta = s.get("color_metadata").or_else(|| s.get("color"));
        let nested = |k: &str| {
            cmeta
                .and_then(|c| c.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let meta = ColorMeta {
            primaries: str_at(&["color_primaries", "color_primary"])
                .or_else(|| nested("primaries")),
            transfer: str_at(&["color_transfer"]).or_else(|| nested("transfer")),
            // ffprobe 用 `color_space` 表示矩阵（bt709 / bt2020nc…），没有单独的 matrix 键
            matrix: str_at(&["color_space"]).or_else(|| nested("matrix")),
            range: str_at(&["color_range"]).or_else(|| nested("range")),
            pix_fmt: str_at(&["pix_fmt"]),
        };
        let hdr_signaled = side_data_signals_hdr(s);
        let duration = s
            .get("duration")
            .and_then(as_numberish_str)
            .and_then(|d| Rational::parse(&d).ok())
            .or_else(|| {
                root.get("format")
                    .and_then(|f| f.get("duration"))
                    .and_then(as_numberish_str)
                    .and_then(|d| Rational::parse(&d).ok())
            });
        out.push(VideoStream {
            index,
            width,
            height,
            r_frame_rate: parse_rate(s.get("r_frame_rate")),
            avg_frame_rate: parse_rate(s.get("avg_frame_rate")),
            duration,
            color: meta,
            hdr_signaled,
        });
    }
    Ok(out)
}

/// `0:00:01.5` 这类时间串不解析（本项目 duration 只用秒）；`"23.976"` 或 `1234.5` 都接受。
fn as_numberish_str(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_rate(v: Option<&serde_json::Value>) -> Option<Rational> {
    let s = match v? {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    Rational::parse(&s).ok()
}

fn side_data_signals_hdr(stream: &serde_json::Value) -> bool {
    let sd = stream.get("side_data_list").and_then(|v| v.as_array());
    let Some(sd) = sd else { return false };
    sd.iter()
        .filter_map(|e| e.get("side_data_type"))
        .filter_map(|v| v.as_str())
        .any(|t| t.contains("HDR") || t.contains("Mastering") || t.contains("Content Light"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UHD_HDR: &str = r#"{
      "format": {"duration": "5400.0"},
      "streams": [
        {"index": 0, "codec_type": "audio", "codec_name": "a"},
        {"index": 1, "codec_type": "video", "width": 3840, "height": 2160,
         "r_frame_rate": "24000/1001", "avg_frame_rate": "24000/1001",
         "pix_fmt": "yuv420p10le", "duration": "5400.0",
         "color_metadata": {"primaries": "bt709", "transfer": "smpte2084", "matrix": "bt2020nc", "range": "tv"},
         "side_data_list": [{"side_data_type": "Mastering display metadata"}]}
      ]
    }"#;

    #[test]
    fn picks_only_video_stream_and_reads_exact_fps() {
        let v = parse_video_streams(UHD_HDR).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].width, v[0].height), (3840, 2160));
        assert_eq!(v[0].fps_candidates(), vec![Fps::NTSC_23976]);
        assert_eq!(
            v[0].r_frame_rate.unwrap(),
            Rational::new(24000, 1001).unwrap()
        );
        assert_eq!(v[0].duration.unwrap(), Rational::from_int(5400));
    }

    #[test]
    fn hdr_is_detected_from_transfer_or_side_data() {
        let v = parse_video_streams(UHD_HDR).unwrap();
        assert!(v[0].is_hdr());
        let only_transfer: &str = r#"{"streams":[{"index":0,"codec_type":"video","width":1920,"height":1080,
          "r_frame_rate":"25/1","color_metadata":{"transfer":"arib-std-b67"}}]}"#;
        assert!(parse_video_streams(only_transfer).unwrap()[0].is_hdr());
        let sdr: &str = r#"{"streams":[{"index":0,"codec_type":"video","width":1920,"height":1080,
          "r_frame_rate":"25/1","color_metadata":{"transfer":"bt709","primaries":"bt709"}}]}"#;
        assert!(!parse_video_streams(sdr).unwrap()[0].is_hdr());
    }

    /// 从**真工具抓下来的输出**解析（本机 gyan ffprobe 5.1/4.1 构建，
    /// `-show_format -show_streams` 的结果原样存在 tests/fixtures/*.ffprobe.json）。
    ///
    /// 这条测试存在的理由：之前所有色彩断言都建立在我自己构想的
    /// `color_metadata:{transfer:…}` 嵌套形状上，单测全绿，而真 ffprobe 给的是扁平的
    /// `color_transfer` —— 于是 HDR 判定在真实世界里完全失效（ISSUES AF1）。
    /// fixture 由工具本身产生，形状就不可能再猜错。
    #[test]
    fn parses_the_captured_real_ffprobe_payload() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let hdr = std::fs::read_to_string(dir.join("uhd23976-hdr.ffprobe.json"))
            .expect("抓下来的真 ffprobe JSON 必须在");
        let v = parse_video_streams(&hdr).unwrap();
        assert_eq!(v.len(), 1, "{:?}", v);
        assert_eq!((v[0].width, v[0].height), (3840, 2160));
        assert_eq!(
            v[0].color.transfer.as_deref(),
            Some("smpte2084"),
            "{:?}",
            v[0].color
        );
        assert_eq!(v[0].color.primaries.as_deref(), Some("bt2020"));
        assert_eq!(v[0].color.matrix.as_deref(), Some("bt2020nc"));
        assert_eq!(v[0].color.range.as_deref(), Some("tv"));
        assert!(v[0].is_hdr(), "PQ 传输特性就是 HDR 信号（16 §2）");
        assert_eq!(
            v[0].r_frame_rate.map(|r| (r.num(), r.den())),
            Some((24000, 1001))
        );
        assert_eq!(
            v[0].avg_frame_rate, v[0].r_frame_rate,
            "这段素材 r 与 avg 相同"
        );

        let sdr = std::fs::read_to_string(dir.join("fhd25.ffprobe.json")).unwrap();
        let s = parse_video_streams(&sdr).unwrap();
        assert!(!s[0].is_hdr(), "bt709 不是 HDR：{:?}", s[0].color);
        assert_eq!(s[0].color.transfer.as_deref(), Some("bt709"));
        assert_eq!(s[0].fps_candidates(), vec![Fps::PAL_25]);
    }

    #[test]
    fn missing_color_block_and_zero_size_are_tolerated() {
        let j: &str = r#"{"streams":[{"index":0,"codec_type":"video","width":0,"height":0,"r_frame_rate":"0/0"},
          {"index":1,"codec_type":"video","width":1280,"height":720,"r_frame_rate":"25/1"}]}"#;
        let v = parse_video_streams(j).unwrap();
        assert_eq!(v.len(), 1, "0 尺寸流应被跳过");
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].color.primaries, None);
        assert_eq!(v[0].duration, None);
        assert_eq!(v[0].fps_candidates(), vec![Fps::PAL_25]);
    }

    #[test]
    fn numeric_and_string_frames_are_both_accepted() {
        let j: &str = r#"{"streams":[{"index":0,"codec_type":"video","width":1920,"height":1080,
          "r_frame_rate": 30, "avg_frame_rate":"30000/1001","duration": 120.5}]}"#;
        let v = parse_video_streams(j).unwrap();
        assert_eq!(v[0].r_frame_rate, Some(Rational::from_int(30)));
        assert_eq!(
            v[0].avg_frame_rate,
            Some(Rational::new(30000, 1001).unwrap())
        );
        assert_eq!(v[0].duration.unwrap(), Rational::new(241, 2).unwrap());
    }

    #[test]
    fn garbage_input_is_error_not_panic() {
        assert!(parse_video_streams("").is_err());
        assert!(parse_video_streams("{\"streams\": {}}").is_err());
        assert_eq!(parse_video_streams("{\"streams\":[]}").unwrap().len(), 0);
        let e = parse_video_streams("{\"nope\":1}").unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0011");
    }
}
