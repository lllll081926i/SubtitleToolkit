//! PNG 侧守卫（13 §7「PNG 实际尺寸与 XML Graphic Width/Height 必须一致」/ AT-BDN-001）。
//!
//! 只读 IHDR，不引图片库：签名 8 + length 4 + `"IHDR"` 4 + width 4 + height 4 = 24 字节，
//! 连 IHDR 余下字段凑 33 字节（`crate::PNG_HEADER_BYTES`）。任何位深/颜色类型/隔行都接受
//! （"非调色板也支持"——我们要的是宽高，不是像素），但签名、chunk 类型与长度必须严格。

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};

use crate::model::BdnDocument;
use crate::PNG_HEADER_BYTES;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const IHDR: &[u8; 4] = b"IHDR";
/// IHDR data 字段的固定长度；不是 13 就不是合法 PNG。
const IHDR_LEN: u32 = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngSize {
    pub width: u32,
    pub height: u32,
}

/// 从 PNG 头部读实际宽高（只看前缀；不校验后续 chunk 的 CRC）。
pub fn png_size(png_bytes: &[u8]) -> Result<PngSize> {
    if png_bytes.len() < 8 {
        return Err(truncated("签名", 8, png_bytes.len()));
    }
    if png_bytes[..8] != SIGNATURE {
        return Err(StkError::simple(Domain::Bdn, 70, "PNG 签名不匹配")
            .with_detail(format!("前 8 字节 {:02X?}", &png_bytes[..8]))
            .with_action("BDN 的 Graphic 必须是 PNG（13 §7）；扩展名不可信，按内容判定"));
    }
    if png_bytes.len() < 16 {
        return Err(truncated("IHDR chunk 头", 16, png_bytes.len()));
    }
    // 旧工程读尺寸时跳过 length/type，遇到带前导 chunk 的文件就会读到别处的字节。
    if &png_bytes[12..16] != IHDR {
        return Err(
            StkError::simple(Domain::Bdn, 72, "PNG 首个 chunk 不是 IHDR")
                .with_detail(format!("{:02X?}", &png_bytes[12..16])),
        );
    }
    let len = be_u32(png_bytes, 8)?;
    if len != IHDR_LEN {
        return Err(StkError::simple(Domain::Bdn, 72, "PNG IHDR 长度非法")
            .with_detail(format!("声明 {len} 字节，规范要求 {IHDR_LEN}")));
    }
    if png_bytes.len() < 24 {
        return Err(truncated("IHDR 宽高字段", 24, png_bytes.len()));
    }
    let width = be_u32(png_bytes, 16)?;
    let height = be_u32(png_bytes, 20)?;
    // Canvas::new 负责 0 边与离谱尺寸（>8192），不在这里另写一套上限。
    Canvas::new(width, height).map_err(|_| {
        StkError::simple(Domain::Bdn, 74, "PNG IHDR 尺寸不合理")
            .with_detail(format!("{width}x{height}"))
    })?;
    Ok(PngSize { width, height })
}

/// 声明尺寸 vs PNG 实际尺寸的唯一比对入口。
pub fn check_png_size(png_bytes: &[u8], declared_w: u32, declared_h: u32) -> Result<()> {
    let actual = png_size(png_bytes)?;
    if actual.width != declared_w || actual.height != declared_h {
        return Err(
            StkError::simple(Domain::Bdn, 73, "XML 声明尺寸与 PNG 实际尺寸不一致")
                .with_detail(format!(
                    "声明 {}x{}，IHDR 实际 {}x{}",
                    declared_w, declared_h, actual.width, actual.height
                ))
                .with_action("Width/Height 必须原样取 PNG 尺寸；min_axis 只是过滤阈值，不能抬尺寸"),
        );
    }
    Ok(())
}

/// 目录里发现的一个问题。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PngProblem {
    /// 出错事件在 `doc.events` 里的下标。
    pub event: usize,
    pub file_name: String,
    /// 失败细节（缺失/非 PNG/截断/尺寸不一致/路径非法）；错误码用 `error.code_string()`。
    pub error: StkError,
}

/// `validate_dir` 的结果：核对过的 PNG 数 + 所有问题。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub checked: usize,
    pub problems: Vec<PngProblem>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// 给 CLI / validator 的一行摘要（22 §诊断）。
    pub fn summary(&self) -> String {
        let head = format!(
            "核对 {} 个 PNG，{} 个问题",
            self.checked,
            self.problems.len()
        );
        match self.problems.first() {
            Some(p) => format!(
                "{head}（首个：events[{}] {} → {}）",
                p.event,
                p.file_name,
                p.error.code_string()
            ),
            None => head,
        }
    }
}

/// 真实读目录里的 PNG，逐个比对 XML 声明尺寸（供 CLI / validator 用）。
///
/// 每个文件只读前 `PNG_HEADER_BYTES` 字节，绝不整文件载入；`FileName` 允许绝对路径
/// （BDN 交换目录里常见），相对路径则相对 `dir`。路径全程 `PathBuf` + Windows verbatim
/// 前缀，支持中文与 >260 字符长路径（23 §文件：不把路径强转 UTF-8 做系统调用）。
/// 单个文件的问题进 `Report.problems`，不中断整体校验。
pub fn validate_dir(doc: &BdnDocument, dir: &Path) -> Result<Report> {
    let mut report = Report::default();
    for (i, ev) in doc.events.iter().enumerate() {
        report.checked = report.checked.saturating_add(1);
        let file = &ev.graphic.file_name;
        let path = match resolve(dir, file) {
            Ok(p) => p,
            Err(e) => {
                report.problems.push(PngProblem {
                    event: i,
                    file_name: file.clone(),
                    error: e,
                });
                continue;
            }
        };
        let mut buf = vec![0u8; PNG_HEADER_BYTES];
        let got = match read_prefix(&path, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                report.problems.push(PngProblem {
                    event: i,
                    file_name: file.clone(),
                    error: io_error(e, file, &path),
                });
                continue;
            }
        };
        // 截断交给 png_size 用同一套 PNG 规则判定（0071），这里不另立标准。
        if let Err(e) = check_png_size(&buf[..got], ev.graphic.width, ev.graphic.height) {
            report.problems.push(PngProblem {
                event: i,
                file_name: file.clone(),
                error: e
                    .with_detail(format!("events[{i}]"))
                    .with_detail(path.display().to_string()),
            });
        }
    }
    Ok(report)
}

fn resolve(dir: &Path, file: &str) -> Result<PathBuf> {
    if file.is_empty() {
        return Err(StkError::simple(Domain::Bdn, 77, "BDN FileName 为空")
            .with_action("BDN 交换目录必须能定位到具体 PNG"));
    }
    if file.chars().any(char::is_control) {
        return Err(StkError::simple(Domain::Bdn, 77, "BDN FileName 含控制字符")
            .with_detail(file.to_string()));
    }
    let p = Path::new(file);
    // 目录穿越一律拒绝：validate_dir 的语义是"校验这个交换目录"，不是任意文件读取器。
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(
            StkError::simple(Domain::Bdn, 77, "BDN FileName 含上级目录成分，拒绝穿越")
                .with_detail(file.to_string()),
        );
    }
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    };
    Ok(verbatim(&joined))
}

/// Windows 长路径：绝对路径加 `\\?\` verbatim 前缀，绕开 MAX_PATH（260）限制（AT-PATH-001）。
fn verbatim(path: &Path) -> PathBuf {
    if !cfg!(windows) || !path.is_absolute() {
        return path.to_path_buf();
    }
    let raw = path.as_os_str().to_string_lossy().to_string();
    if raw.starts_with(r"\\?\") {
        return PathBuf::from(raw);
    }
    if let Some(unc) = raw.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{unc}"));
    }
    PathBuf::from(format!(r"\\?\{}", raw.replace('/', "\\")))
}

/// 返回实际读到的字节数；文件不存在由调用方翻成 `STK-BDN-0075`。
fn read_prefix(path: &Path, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut f = std::fs::File::open(path)?;
    let mut got = 0usize;
    while got < buf.len() {
        let n = f.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got = got.checked_add(n).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "PNG 头部长度溢出")
        })?;
    }
    Ok(got)
}

fn io_error(e: std::io::Error, file: &str, path: &Path) -> StkError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return StkError::simple(Domain::Bdn, 75, "BDN 引用的 PNG 文件不存在")
            .with_detail(file.to_string())
            .with_detail(path.display().to_string())
            .with_action("BDN 交换目录必须整目录交付（XML + 全部 PNG）");
    }
    StkError::simple(Domain::Bdn, 76, "PNG 读取失败")
        .with_detail(file.to_string())
        .with_detail(path.display().to_string())
        .with_detail(format!("{e}"))
}

fn truncated(what: &str, need: usize, got: usize) -> StkError {
    StkError::simple(Domain::Bdn, 71, "PNG 头部截断")
        .with_detail(format!("读 {what} 需要 {need} 字节，实际 {got}"))
}

/// 大端 u32；偏移与长度全部 checked，越界报 0071 而不是 panic。
fn be_u32(b: &[u8], at: usize) -> Result<u32> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| truncated("chunk 字段", usize::MAX, b.len()))?;
    if end > b.len() {
        return Err(truncated("chunk 字段", end, b.len()));
    }
    let s = &b[at..end];
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
