//! 多输出共享缓存 key（15 §4）与外部版本失效（38 §6）。
//!
//! key 必须同时包含：输入哈希 + 变换哈希 + canvas + fps + renderer 身份 +
//! 字体环境哈希 + 渲染选项。少任何一个，"复用缓存"就变成"复用别的任务的像素"——
//! 把 A 的字幕图当成 B 的，是这类工具最危险的错误。
//!
//! 编码方式：每个字段都带**名字和长度**再喂进 SHA-256。裸拼接会让 `("ab","c")` 与
//! `("a","bc")` 撞 key，这种绑定歧义在哈希合同里是真实存在过的 bug 类型。

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::time::Fps;
use sha2::{Digest as _, Sha256};

/// renderer 身份（12 §4）：不是"文件名像 libass"就算数，要带实际二进制哈希。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderIdentity {
    pub provider: String,
    pub version: String,
    /// provider DLL/EXE 的 SHA-256（64 位小写 hex）
    pub binary_sha256: String,
    pub arch: String,
}

impl RenderIdentity {
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("provider", &self.provider),
            ("version", &self.version),
            ("binary_sha256", &self.binary_sha256),
            ("arch", &self.arch),
        ] {
            if value.trim().is_empty() {
                return Err(
                    StkError::simple(Domain::Dependency, 30, "renderer 身份字段不能为空")
                        .with_detail(format!("RenderIdentity.{field}")),
                );
            }
        }
        if !is_sha256_hex(&self.binary_sha256) {
            return Err(StkError::simple(
                Domain::Dependency,
                31,
                "renderer 二进制哈希必须是 64 位小写 hex",
            )
            .with_action("没有实际二进制哈希就不许声称 renderer 身份（12 §4）"));
        }
        Ok(())
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[derive(Clone, Debug)]
pub struct CacheKeyParts<'a> {
    pub input_sha256: &'a str,
    pub transform_sha256: &'a str,
    pub canvas: Canvas,
    pub fps: Fps,
    pub renderer: &'a RenderIdentity,
    pub font_env_sha256: &'a str,
    pub render_options_sha256: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheKey(pub String);

impl CacheKey {
    pub fn compute(parts: &CacheKeyParts<'_>) -> Result<CacheKey> {
        parts.renderer.validate()?;
        for (field, value) in [
            ("input_sha256", parts.input_sha256),
            ("transform_sha256", parts.transform_sha256),
            ("font_env_sha256", parts.font_env_sha256),
            ("render_options_sha256", parts.render_options_sha256),
        ] {
            if !is_sha256_hex(value) {
                return Err(StkError::simple(
                    Domain::Dependency,
                    32,
                    "缓存 key 分量必须是 64 位小写 sha256 hex",
                )
                .with_detail(format!("{field} = {value:?}")));
            }
        }
        let mut h = Sha256::new();
        // 名字与值都带长度前缀：拼接歧义在这里被结构性排除
        let mut field = |name: &[u8], value: &[u8]| {
            h.update((name.len() as u32).to_be_bytes());
            h.update(name);
            h.update((value.len() as u32).to_be_bytes());
            h.update(value);
        };
        field(b"input", parts.input_sha256.as_bytes());
        field(b"transform", parts.transform_sha256.as_bytes());
        field(
            b"canvas",
            format!("{}x{}", parts.canvas.width, parts.canvas.height).as_bytes(),
        );
        let r = parts.fps.as_rational();
        field(b"fps", format!("{}/{}", r.num(), r.den()).as_bytes());
        field(b"renderer.provider", parts.renderer.provider.as_bytes());
        field(b"renderer.version", parts.renderer.version.as_bytes());
        field(b"renderer.binary", parts.renderer.binary_sha256.as_bytes());
        field(b"renderer.arch", parts.renderer.arch.as_bytes());
        field(b"font_env", parts.font_env_sha256.as_bytes());
        field(b"render_options", parts.render_options_sha256.as_bytes());
        Ok(CacheKey(
            h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        ))
    }
}

/// 依赖版本变了（38 §6）：分量不同就不能复用缓存。
pub fn reuse_allowed(want: &CacheKeyParts<'_>, stored_key: &CacheKey) -> Result<bool> {
    let fresh = CacheKey::compute(want)?;
    Ok(fresh == *stored_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::time::Fps;

    const H_E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const H_F: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    const H_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const H_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const H_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const H_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const H_9: &str = "9999999999999999999999999999999999999999999999999999999999999999";

    fn ident() -> RenderIdentity {
        RenderIdentity {
            provider: "libass".into(),
            version: "0.17.3".into(),
            binary_sha256: H_A.into(),
            arch: "x86_64".into(),
        }
    }

    fn parts<'a>(
        input: &'a str,
        canvas: Canvas,
        fps: Fps,
        r: &'a RenderIdentity,
    ) -> CacheKeyParts<'a> {
        CacheKeyParts {
            input_sha256: input,
            transform_sha256: H_B,
            canvas,
            fps,
            renderer: r,
            font_env_sha256: H_C,
            render_options_sha256: H_D,
        }
    }

    #[test]
    fn key_changes_when_any_component_changes() {
        let r = ident();
        let base = CacheKey::compute(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r)).unwrap();
        let mut v_new = ident();
        v_new.version = "0.17.4".into();
        let mut b_new = ident();
        b_new.binary_sha256 = H_9.to_string();
        let cases: Vec<(&str, CacheKeyParts<'_>)> = vec![
            ("输入不同", parts(H_F, Canvas::FHD1080, Fps::PAL_25, &r)),
            (
                "renderer 版本不同",
                parts(H_E, Canvas::FHD1080, Fps::PAL_25, &v_new),
            ),
            (
                "renderer 二进制不同",
                parts(H_E, Canvas::FHD1080, Fps::PAL_25, &b_new),
            ),
            ("画布不同", parts(H_E, Canvas::HD720, Fps::PAL_25, &r)),
            ("fps 不同", parts(H_E, Canvas::FHD1080, Fps::NTSC_23976, &r)),
        ];
        for (what, p) in cases {
            let k = CacheKey::compute(&p).unwrap();
            assert_ne!(base, k, "{what} 必须让缓存 key 变化");
        }
    }

    #[test]
    fn compute_is_reproducible() {
        let r = ident();
        let a = CacheKey::compute(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r)).unwrap();
        let b = CacheKey::compute(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r)).unwrap();
        assert_eq!(a, b, "同样的分量必须给出同样的 key");
        assert_eq!(a.0.len(), 64);
    }

    #[test]
    fn rejects_missing_or_uppercase_binary_hash() {
        let mut r = ident();
        r.binary_sha256 = H_A.to_uppercase();
        let e = CacheKey::compute(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r)).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0031");
        r.binary_sha256 = H_A.to_string();
        r.provider = " ".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn component_must_be_sha256_hex() {
        let r = ident();
        let mut p = parts("short", Canvas::FHD1080, Fps::PAL_25, &r);
        let e = CacheKey::compute(&p).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0032");
        p.input_sha256 = H_E;
        assert!(CacheKey::compute(&p).is_ok());
    }

    #[test]
    fn reuse_gate_reports_no_match_instead_of_guessing() {
        let r = ident();
        let stored = CacheKey::compute(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r)).unwrap();
        assert!(reuse_allowed(&parts(H_E, Canvas::FHD1080, Fps::PAL_25, &r), &stored).unwrap());
        assert!(!reuse_allowed(&parts(H_F, Canvas::FHD1080, Fps::PAL_25, &r), &stored).unwrap());
    }
}
