//! host ↔ 主进程的控制面协议（12 §2 的 handshake / 请求 / 帧传输）。
//!
//! 每条消息：`1 字节标签 + u32 小端长度 + 负载`。标签 `1` 是 JSON（UTF-8），
//! `2` 是原始字节（脚本字节、RGBA 帧）。两种负载共用同一套分帧，是为了让
//! "请求后面必须跟一块数据"这种顺序关系在流上明确可查，而不是塞进 JSON 里
//! 再 base64 一遍（一个大字节的 ASS 脚本与一帧 1080p RGBA 都不该被膨胀 33%）。
//!
//! 为什么用子进程的 stdin/stdout 而不是**命名管道**：12 §2 要的是"控制面走管道"，
//! 匿名管道同样满足，而且天然只有父子双方能连；命名管道要额外的 ACL 设计，
//! 在没有 GUI 多进程需求之前那是净增的攻击面（23 §安全）。差异记在 ISSUES AE1。

use std::io::{self, Read, Write};

use core_model::loss::LossReport;
use serde::{Deserialize, Serialize};

/// 协议版本：主进程与 host 必须一致，否则直接拒绝而不是"尽力而为"。
pub const API_VERSION: u32 = 2;

pub const TAG_TEXT: u8 = 1;
pub const TAG_BLOB: u8 = 2;

/// 单条消息的上限。帧本身（1080p RGBA = 8 337 920 字节）远小于它，
/// 而恶意/错乱的长度字段绝不允许换来一次巨型分配（25 AT-SEC-001 的同一纪律）。
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// 崩溃注入的测试通道开关。只有宿主进程的**环境变量**能让宿主自杀，
/// 控制面协议本身不再携带任何自毁指令（2026-10-09 审查：正式二进制不得留有
/// 可被客户端控制的 abort/hang 后门）。AT-RENDER-002 的崩溃用例经此开关触发。
pub const CRASH_ENV: &str = "STK_HOST_CRASH";

/// 22 §6：崩溃现场记录写到哪个目录。
///
/// 与 `CRASH_ENV` 同一类（**启动环境**而不是控制面）：客户端无法在运行时
/// 把它改到别处，也就不存在"让 host 往任意目录写文件"这条路径。
pub const CRASH_DIR_ENV: &str = "STK_HOST_CRASH_DIR";
pub const FRAME_CHUNK_BYTES: usize = 16 * 1024 * 1024;

/// 握手结果：12 §2 要求逐项交代 provider 类型、名字、版本、PE 位数、API 版本、像素格式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    /// 12 §1 的 renderer 家族名：当前是 `libass` 或 `csri`。
    ///
    /// 它是**按导出表判出来的**，不按文件名猜（那正是 12 §1 的要求）。
    /// 客户端要拒绝的是“我不认识的名字”，而不是“不是 libass”——
    /// 后者会把 CSRI 链当成协议不符。
    pub provider: String,
    /// DLL 的文件名，仅供人读；身份以哈希为准
    pub name: String,
    /// libass 自己的版本号（`ass_library_version()`）的十六进制文本
    pub version: String,
    pub api_version: u32,
    /// host 可执行文件的位数
    pub host_arch: String,
    /// provider DLL 的位数（12 §3：必须与 host 一致）
    pub provider_arch: String,
    /// 帧负载的像素格式：固定 RGBA8 行主序
    pub pixel_format: String,
    /// provider 二进制的 SHA-256（12 §4：身份是哈希，不是路径）
    pub provider_binary_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// 第一条消息；host 在此之前不加载任何 DLL。
    Hello {
        dll: String,
        /// 脚本的 PlayRes 空间（09 §2 的 storage）
        storage: [u32; 2],
        /// 输出画布
        output: [u32; 2],
        default_font: String,
    },
    /// 后面必须紧跟一条 `TAG_BLOB`，内容是 ASS 脚本字节。
    LoadScript,
    /// 必须在 LoadScript 之前发送，后面紧跟一条字体字节负载。
    AddFont {
        family: String,
    },
    Render {
        /// 整数毫秒：libass 的 `ass_render_frame` 只接受整毫秒
        pts_ms: i64,
    },
    Bye,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Hello(Handshake),
    ScriptLoaded {
        bytes: usize,
    },
    FontAdded {
        bytes: usize,
    },
    /// 后面必须紧跟一条 `TAG_BLOB`，内容是 RGBA（长度 = width*height*4）。
    /// `digest` 是那份 RGBA 的 SHA-256：主进程可以先核摘要再信像素（12 §8 的去重纪律）。
    Frame {
        width: u32,
        height: u32,
        bytes: usize,
        digest: String,
        /// libass 的 `detect_change` 原值：只是提示，不作为唯一去重依据（12 §8）
        detect_change: i32,
        images: usize,
        loss: LossReport,
    },
    /// host 出错：错误码、标题与**具体原因**一起带回主进程
    /// （12 §5 "provider load failure 独立错误码"）。
    ///
    /// `details` 不能省：host 是唯一真的看过那个 DLL 的地方，
    /// 它知道"这是一个 CSRI provider，不是 libass"或"缺哪几个符号"。
    /// 只传一个概括的 `title` 会让那份诊断在进程边界上丢掉。
    /// 旧版协议没这个字段，所以用 `#[serde(default)]` 保持向后兼容。
    Error {
        code: String,
        title: String,
        #[serde(default)]
        details: Vec<String>,
    },
    ByeOk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Blob(Vec<u8>),
}

pub fn write_message(w: &mut impl Write, msg: &Message) -> io::Result<()> {
    let (tag, body): (u8, &[u8]) = match msg {
        Message::Text(t) => (TAG_TEXT, t.as_bytes()),
        Message::Blob(b) => (TAG_BLOB, b.as_slice()),
    };
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("单条消息 {} 字节，超过上限 {MAX_MESSAGE_BYTES}", body.len()),
        ));
    }
    let len = (body.len() as u32).to_le_bytes();
    w.write_all(&[tag])?;
    w.write_all(&len)?;
    w.write_all(body)?;
    w.flush()
}

/// 干净的 EOF 返回 `Ok(None)`（对端退出/被关闭）；截断或长度非法返回错误。
pub fn read_message(r: &mut impl Read) -> io::Result<Option<Message>> {
    let mut header = [0u8; 5];
    let mut got = 0usize;
    while got < header.len() {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("消息头被截断：只读到 {got}/5 字节"),
                ))
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    if !matches!(header[0], TAG_TEXT | TAG_BLOB) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("未知的消息标签 {}", header[0]),
        ));
    }
    let len = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("对端声明消息长度 {len} 字节，超过上限 {MAX_MESSAGE_BYTES}"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    match header[0] {
        TAG_TEXT => String::from_utf8(body)
            .map(|text| Some(Message::Text(text)))
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("协议文本不是合法 UTF-8：{e}"),
                )
            }),
        TAG_BLOB => Ok(Some(Message::Blob(body))),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("未知的消息标签 {other}"),
        )),
    }
}

pub fn to_json<T: Serialize>(v: &T) -> io::Result<Message> {
    serde_json::to_string(v)
        .map(Message::Text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))
}

pub fn from_json<T: for<'de> Deserialize<'de>>(msg: &Message) -> io::Result<T> {
    let Message::Text(text) = msg else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "期望 JSON 消息，收到原始字节",
        ));
    };
    serde_json::from_str::<T>(text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("协议 JSON 非法：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_blob_frames_round_trip_exactly() {
        let req = Request::Hello {
            dll: "C:\\x\\ass-9.dll".to_string(),
            storage: [1920, 1080],
            output: [1920, 1080],
            default_font: "Arial".to_string(),
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &to_json(&req).unwrap()).unwrap();
        let blob = Message::Blob(vec![7u8; 4096]);
        write_message(&mut buf, &blob).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let back: Request = from_json(&read_message(&mut cursor).unwrap().unwrap()).unwrap();
        assert_eq!(back, req);
        assert_eq!(
            read_message(&mut cursor).unwrap().unwrap(),
            blob,
            "原始字节负载必须原样回来，不能被当成文本处理"
        );
        assert_eq!(read_message(&mut cursor).unwrap(), None, "干净 EOF 是 None");
    }

    #[test]
    fn absurd_declared_length_is_refused_before_allocating_it() {
        // 标签 + 一个几乎满是 1 的长度字段：读侧必须在分配之前拒绝
        let mut evil = vec![TAG_BLOB];
        evil.extend_from_slice(&u32::MAX.to_le_bytes());
        evil.extend_from_slice(b"tail");
        let err = read_message(&mut std::io::Cursor::new(evil)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        assert!(err.to_string().contains("超过上限"), "{err}");
    }

    #[test]
    fn truncated_header_is_an_error_not_a_hang_or_a_panic() {
        let err = read_message(&mut std::io::Cursor::new(vec![TAG_TEXT, 3]))
            .unwrap_err()
            .kind();
        assert_eq!(err, io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn unknown_tag_is_rejected() {
        let mut buf = vec![9u8];
        buf.extend_from_slice(&(MAX_MESSAGE_BYTES as u32).to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let err = read_message(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("未知的消息标签"), "{err}");
        assert_eq!(cursor.position(), 5, "未知标签不得读取或分配负载");
    }

    #[test]
    fn invalid_utf8_is_rejected_without_replacing_bytes() {
        let bytes = [TAG_TEXT, 3, 0, 0, 0, b'"', 0xff, b'"'];
        let err = read_message(&mut std::io::Cursor::new(bytes)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("UTF-8"), "{err}");
    }

    #[test]
    fn api_version_is_part_of_the_handshake() {
        let hs = Handshake {
            provider: "libass".to_string(),
            name: "ass-9.dll".to_string(),
            version: "0x01705000".to_string(),
            api_version: API_VERSION,
            host_arch: "x64".to_string(),
            provider_arch: "x64".to_string(),
            pixel_format: "RGBA8".to_string(),
            provider_binary_sha256: "0".repeat(64),
        };
        assert_eq!(hs.api_version, 2, "协议版本变了就必须同时改两侧");
    }
}
