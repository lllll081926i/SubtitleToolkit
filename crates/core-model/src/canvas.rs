//! 画布（09_CANVAS_RESOLUTION_MULTI_OUTPUT.md 的硬规则：特效字幕永远是完整 16:9）。

use crate::error::{Domain, StkError};
use crate::time::Rational;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

/// Blu-ray / UHD-Bud 规范内 PGS 的最大画布（2K）。3840×2160 PGS 绝不标 compliant。
pub const BLURAY_MAX: Canvas = Canvas {
    width: 1920,
    height: 1080,
};

impl Canvas {
    pub const HD720: Canvas = Canvas {
        width: 1280,
        height: 720,
    };
    pub const FHD1080: Canvas = Canvas {
        width: 1920,
        height: 1080,
    };
    pub const UHD2160: Canvas = Canvas {
        width: 3840,
        height: 2160,
    };

    pub const PRESETS: &'static [Canvas] = &[Canvas::HD720, Canvas::FHD1080, Canvas::UHD2160];

    pub fn new(width: u32, height: u32) -> Result<Canvas, StkError> {
        if width == 0 || height == 0 {
            return Err(StkError::simple(Domain::Input, 30, "画布尺寸必须为正"));
        }
        // 限制到 8K，避免明显异常的坐标规模进入位图分配
        if width > 8192 || height > 8192 {
            return Err(StkError::simple(Domain::Input, 30, "画布尺寸超出支持上限"));
        }
        Ok(Canvas { width, height })
    }

    /// 严格 16:9：用乘法判定，不用浮点比值。
    pub fn is_16_9(self) -> bool {
        u64::from(self.width) * 9 == u64::from(self.height) * 16
    }

    /// PlayRes 组合：只接受恰好 16:9（09 §1 的全局硬规则）。
    pub fn strict_16_9(self) -> Result<Canvas, StkError> {
        if self.is_16_9() {
            Ok(self)
        } else {
            Err(
                StkError::simple(Domain::Input, 31, "特效字幕逻辑画布必须是完整 16:9")
                    .with_detail(format!("{}x{}", self.width, self.height))
                    .with_action("改用 1280x720 / 1920x1080 / 3840x2160，或显式声明为非特效输出"),
            )
        }
    }

    /// 整数倍缩放系数；比例不一致时报错而不是静默各轴缩放。
    pub fn scale_factor_to(self, target: Canvas) -> Result<Rational, StkError> {
        if !self.is_16_9() || !target.is_16_9() {
            return Err(StkError::simple(
                Domain::Transform,
                32,
                "只有 16:9 画布之间允许等比缩放",
            ));
        }
        let f = Rational::new(i64::from(target.width), i64::from(self.width))
            .map_err(|_| StkError::simple(Domain::Transform, 32, "缩放系数无效"))?;
        let fy = Rational::new(i64::from(target.height), i64::from(self.height))
            .map_err(|_| StkError::simple(Domain::Transform, 32, "缩放系数无效"))?;
        if f != fy {
            return Err(StkError::simple(
                Domain::Transform,
                32,
                "宽高缩放系数不一致",
            ));
        }
        Ok(f)
    }

    /// PGS 是否落在 Blu-ray 允许的画布内（≤ 1920×1080）。
    pub fn within_bluray(self) -> bool {
        self.width <= BLURAY_MAX.width && self.height <= BLURAY_MAX.height
    }

    pub fn label(self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

impl core::fmt::Display for Canvas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_nine_is_exact() {
        for c in Canvas::PRESETS {
            assert!(c.is_16_9(), "{c}");
        }
        assert!(!Canvas {
            width: 1920,
            height: 800
        }
        .is_16_9());
        assert!(!Canvas {
            width: 2560,
            height: 1080
        }
        .is_16_9());
        assert!(Canvas::strict_16_9(Canvas::UHD2160).is_ok());
        assert!(Canvas::strict_16_9(Canvas {
            width: 1920,
            height: 808
        })
        .is_err());
        assert!(Canvas::new(0, 1080).is_err());
        assert!(Canvas::new(100_000, 56_250).is_err());
    }

    #[test]
    fn scale_factor_is_rational_and_consistent() {
        assert_eq!(
            Canvas::FHD1080.scale_factor_to(Canvas::UHD2160).unwrap(),
            Rational::from_int(2)
        );
        assert_eq!(
            Canvas::UHD2160.scale_factor_to(Canvas::FHD1080).unwrap(),
            Rational::new(1, 2).unwrap()
        );
        assert_eq!(
            Canvas::HD720.scale_factor_to(Canvas::FHD1080).unwrap(),
            Rational::new(3, 2).unwrap()
        );
        assert!(Canvas {
            width: 1920,
            height: 1080
        }
        .scale_factor_to(Canvas {
            width: 1280,
            height: 800
        })
        .is_err());
    }

    #[test]
    fn bluray_bounds_reject_2160_pgs() {
        assert!(Canvas::FHD1080.within_bluray());
        assert!(!Canvas::UHD2160.within_bluray());
        assert_eq!(BLURAY_MAX, Canvas::FHD1080);
    }
}
