//! 测试公共件：PNG 头部构造器 + BDN 样例。
//!
//! 只做"能让 IHDR 被读出来"的最小 PNG：签名 + IHDR（含正确 CRC）+ IEND。
//! 不引图片库（依赖决定），因此这里自带 crc32。
#![allow(dead_code)]

use codec_bdn::{BdnGraphic, BdnMeta};
use core_model::time::{Fps, Time};

/// 常用文档头（1080p、给定帧率）。
pub fn meta(fps: Fps) -> BdnMeta {
    BdnMeta {
        version: "0.93".to_string(),
        name: "测试".to_string(),
        language: "chi".to_string(),
        video_format: "1080p".to_string(),
        frame_rate: fps.as_rational(),
        drop_frame: false,
        width: 1920,
        height: 1080,
    }
}

/// 一个 1 秒长、位置合法的事件。
pub fn event(i: usize, start_ms: i64, fps: Fps) -> codec_bdn::BdnEvent {
    let start = Time::from_millis(start_ms).unwrap();
    let end = Time::from_millis(start_ms + 1000).unwrap();
    let g = BdnGraphic::new(format!("{i:05}.png"), 800, 120, 560, 880).unwrap();
    codec_bdn::BdnEvent::new(start, end, g, fps).unwrap()
}

/// 给定起点列表造文档（必须严格递增）。
pub fn doc(fps: Fps, starts_ms: &[i64]) -> codec_bdn::BdnDocument {
    let events: Vec<_> = starts_ms
        .iter()
        .enumerate()
        .map(|(i, ms)| event(i, *ms, fps))
        .collect();
    codec_bdn::BdnDocument::new(meta(fps), events)
}

pub const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// core-model 的 `Rational::abs` 是它自己的测试辅助，不在公开 API 里。
pub fn abs(r: core_model::time::Rational) -> core_model::time::Rational {
    if r.num() < 0 {
        core_model::time::Rational::new(-r.num(), r.den()).unwrap()
    } else {
        r
    }
}

/// 最小合法 PNG：`w x h`、8-bit RGBA（非调色板），带真实 CRC。
pub fn png(w: u32, h: u32) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color type RGBA, compr, filter, interlace
    let mut out = Vec::new();
    out.extend_from_slice(&PNG_SIG);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(
        &mut out,
        b"IDAT",
        &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00],
    );
    chunk(&mut out, b"IEND", &[]);
    out
}

/// 只写签名 + IHDR 前缀的"截断 PNG"，用于测截断路径。
pub fn png_truncated(w: u32, h: u32, keep: usize) -> Vec<u8> {
    let full = png(w, h);
    full[..keep.min(full.len())].to_vec()
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// 一个 1080p、23.976 的两事件 BDN（SupRip 方言：`<Graphic>` 子元素）。
pub fn sample_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<BDN Version="0.93" Xml:lang="chi">
<BDNFormat>
<VideoFormat>1080p</VideoFormat>
<FrameRate>23.976</FrameRate>
<DropFrame>FALSE</DropFrame>
<DisplayResolution><Width>1920</Width><Height>1080</Height></DisplayResolution>
<Name>中文字幕 &amp; 测试</Name>
</BDNFormat>
<BDNSubtitleData>
<Event forced="false" InTC="00:00:01:00" OutTC="00:00:03:12">
<Graphic>
<FileName>0001.png</FileName>
<Width>800</Width><Height>120</Height><X>560</X><Y>880</Y>
</Graphic>
</Event>
<Event forced="false" InTC="00:00:03:12" OutTC="00:00:07:00">
<Graphic>
<FileName>字幕 中文.png</FileName>
<Width>100</Width><Height>50</Height><X>10</X><Y>20</Y>
</Graphic>
</Event>
</BDNSubtitleData>
</BDN>
"#
    .to_string()
}

/// 同样内容、Subtitle Edit 方言（时间码/几何走属性，Graphic 裸文本是文件名）。
pub fn sample_xml_dialect_attrs() -> String {
    r#"<bdn version="0_r22" xml:lang="chi">
<BDNSUBTITLEDATA>
  <EVENT InTC="00:00:01:00" OutTC="00:00:03:12" Path="a b.png" Width="800" Height="120" X="560" Y="880" />
</BDNSUBTITLEDATA>
</bdn>
"#
    .to_string()
}
