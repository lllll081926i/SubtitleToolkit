//! render-api 入口。
//! 本 crate 只有类型与协议，不含任何 native 依赖：真正的 libass/CSRI 绑定在
//! render-libass / render-host-* 里实现，第三方 DLL 永远不进 GUI 进程（ADR-005）。

pub mod capability;
pub mod timeline;

use core_model::{Canvas, Fps, Result, StkError, Time};

/// 一帧 BGRA（与 Windows DIB/libass ASS_IMAGE 常见的字节序一致）。
/// `w*h*4` 必须等于 `pixels.len()`，由 `Frame::new` 保证。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub pts: Time,
    pub pixels: Vec<u8>,
}

impl Frame {
    pub fn new(w: u32, h: u32, pts: Time) -> Result<Frame> {
        let n = (w as usize)
            .checked_mul(h as usize)
            .and_then(|a| a.checked_mul(4));
        let Some(n) = n else {
            return Err(StkError::simple(
                core_model::Domain::Render,
                1,
                "帧尺寸乘法溢出",
            ));
        };
        Ok(Frame {
            w,
            h,
            pts,
            pixels: vec![0u8; n],
        })
    }
    pub fn check_len(&self) -> Result<()> {
        let expect = (self.w as usize)
            .checked_mul(self.h as usize)
            .and_then(|a| a.checked_mul(4));
        if expect != Some(self.pixels.len()) {
            return Err(StkError::simple(
                core_model::Domain::Render,
                2,
                "帧像素缓冲长度与 w*h*4 不一致",
            )
            .with_detail(format!(
                "{}x{} expect={:?} actual={}",
                self.w,
                self.h,
                expect,
                self.pixels.len()
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    pub provider: String,
    pub version: String,
    pub arch: Arch,
    /// 能否逐时间精确取帧（而非整段解码）
    pub random_access: bool,
    /// 是否支持 VSFilterMod 私有特效
    pub vsfiltermod_extensions: bool,
    pub thread_safe: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Arch {
    X64,
    X86,
    Unknown,
}

/// 字体环境指纹（17 §7：render cache key 必须含字体环境 hash）。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FontContext {
    pub provider: String,
    pub env_hash: String,
    pub extra_dirs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RenderRequest {
    /// ASS 脚本内容（宿主不接受未解析的裸路径，避免 host 侧再读盘）
    pub script: String,
    pub canvas: Canvas,
    pub fps: Fps,
    pub fonts: FontContext,
}

/// 规格里 renderer 的三件事：open / render / capabilities。
pub trait Renderer {
    fn capabilities(&self) -> Result<Capabilities>;
    fn open(&mut self, req: &RenderRequest) -> Result<()>;
    fn render(&mut self, at: Time) -> Result<Frame>;
    fn close(&mut self) -> Result<()>;
}

/// host 生命周期状态（38 §3：崩溃只终止 host，GUI 不跟随死）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostState {
    Idle,
    Starting,
    Ready,
    Crashed,
    Exited,
}

/// IPC：长度前缀 + JSON envelope（38/12 §2）。大帧数据走共享内存，
/// 这里只描述元数据句柄，避免把协议字节布局硬编码进核心。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMsg {
    Hello {
        protocol: u32,
        request: RenderRequest,
    },
    Ready {
        caps: Capabilities,
    },
    RenderAt {
        seq: u64,
        time: Time,
    },
    Frame {
        seq: u64,
        shm_name: String,
        w: u32,
        h: u32,
        bytes: u64,
    },
    Cancel {
        seq: u64,
    },
    Bye,
    Error {
        seq: u64,
        code: String,
        title: String,
    },
}

pub const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::{Fps, Time};

    fn canvas() -> Canvas {
        Canvas::FHD1080
    }

    #[test]
    fn frame_len_matches_and_is_checked() {
        let f = Frame::new(4, 2, Time::ZERO).unwrap();
        assert_eq!(f.pixels.len(), 4 * 2 * 4);
        assert!(f.check_len().is_ok());
        let mut bad = f.clone();
        bad.pixels.truncate(10);
        let e = bad.check_len().unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0002");
    }

    #[test]
    fn frame_size_overflow_is_caught() {
        assert!(Frame::new(u32::MAX, u32::MAX, Time::ZERO).is_err());
    }

    #[test]
    fn host_msg_roundtrips_json() {
        let req = RenderRequest {
            script: "[Script Info]\nPlayResX: 1920\n".to_string(),
            canvas: canvas(),
            fps: Fps::NTSC_23976,
            fonts: FontContext {
                provider: "directwrite".into(),
                env_hash: "abc".into(),
                extra_dirs: vec![],
            },
        };
        let msg = HostMsg::Hello {
            protocol: PROTOCOL_VERSION,
            request: req,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: HostMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
        assert!(json.contains("\"type\":\"hello\""));
    }

    #[test]
    fn frame_message_canonicalizes_time_as_rational() {
        let m = HostMsg::RenderAt {
            seq: 3,
            time: Time::from_millis(1500).unwrap(),
        };
        let json = serde_json::to_string(&m).unwrap();
        // 不允许出现 float 秒：应以 num/den 形式落到协议里
        assert!(
            json.contains("\"num\":3") && json.contains("\"den\":2"),
            "{json}"
        );
        let back: HostMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn capabilities_default_shape() {
        let c = Capabilities {
            provider: "libass".into(),
            version: "0.17.5".into(),
            arch: Arch::X64,
            random_access: true,
            vsfiltermod_extensions: false,
            thread_safe: true,
        };
        assert_eq!(c.arch, Arch::X64);
        assert!(!c.vsfiltermod_extensions);
    }
}
