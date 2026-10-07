//! V3.1 分层坐标模型（09 §2、40 §2）。四个概念不得互相偷代：
//! OutputCanvas（输出画布，永远完整 16:9）、ScriptSize（ASS PlayResX/Y）、
//! LayoutRes（可选，libass 对相关计算有优先级）、StorageSize + PixelAspect。

use crate::error::{Domain, StkError};
use crate::time::Rational;

use crate::canvas::Canvas;

/// 脚本分辨率处理策略（09 §4）。默认 Preserve，即"渲染时升采样，不改脚本文本"。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptResolutionPolicy {
    #[default]
    Preserve,
    Rewrite,
}

/// LayoutRes 在 Rewrite 模式下的处理（09 §4 B：不能无脑一起 ×2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutResPolicy {
    /// 保留原值（默认，避免双重缩放）
    #[default]
    KeepAsIs,
    /// 与 PlayRes 一起缩放，必须经过 differential corpus 证明
    ScaleWithScript,
    /// 删除该字段（交由 libass 回退到 PlayRes）
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptSize {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LayoutRes {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StorageSize {
    pub width: u32,
    pub height: u32,
}

impl StorageSize {
    pub fn new(width: u32, height: u32) -> Result<StorageSize, StkError> {
        if width == 0 || height == 0 {
            return Err(StkError::simple(Domain::Render, 10, "storage size 必须为正")
                .with_action("libass 要求显式 ass_set_storage_size；缺失时只能猜测，3D transform/source aspect/blur scale 可能错误"));
        }
        Ok(StorageSize { width, height })
    }
}

/// 像素宽高比，有理数（方形像素 = 1/1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PixelAspect(Rational);

impl PixelAspect {
    pub fn square() -> PixelAspect {
        PixelAspect(Rational::ONE)
    }
    pub fn new(r: Rational) -> Result<PixelAspect, StkError> {
        if r <= Rational::ZERO {
            return Err(StkError::simple(Domain::Input, 33, "pixel aspect 必须为正"));
        }
        Ok(PixelAspect(r))
    }
    pub fn as_rational(&self) -> Rational {
        self.0
    }
    pub fn is_square(&self) -> bool {
        self.0 == Rational::ONE
    }
}

/// 一次输出的完整几何描述。缺少 storage 时不得进入渲染（40 §2.1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub canvas: Canvas,
    pub canvas_is_custom: bool,
    pub script: Option<ScriptSize>,
    pub layout: Option<LayoutRes>,
    pub storage: Option<StorageSize>,
    pub par: PixelAspect,
    pub script_policy: ScriptResolutionPolicy,
    pub layout_policy: LayoutResPolicy,
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry {
            canvas: Canvas::FHD1080,
            canvas_is_custom: false,
            script: None,
            layout: None,
            storage: None,
            par: PixelAspect::square(),
            script_policy: ScriptResolutionPolicy::Preserve,
            layout_policy: LayoutResPolicy::KeepAsIs,
        }
    }
}

impl Geometry {
    pub fn for_canvas(canvas: Canvas) -> Geometry {
        Geometry {
            canvas,
            ..Default::default()
        }
    }

    /// 09 §1：输出画布必须完整 16:9，除非用户明确选择实验性自定义。
    pub fn validate(&self) -> Result<(), StkError> {
        if !self.canvas.is_16_9() && !self.canvas_is_custom {
            return Err(StkError::simple(
                Domain::Input,
                31,
                "输出画布必须是完整 16:9，或显式声明为实验性自定义",
            )
            .with_detail(self.canvas.label()));
        }
        if let (Some(s), false) = (self.script, self.canvas_is_custom) {
            if s.x == 0 || s.y == 0 {
                return Err(StkError::simple(Domain::Parse, 38, "PlayRes 字段为 0"));
            }
        }
        Ok(())
    }

    /// 渲染前检查（40 §2.1：storage 是硬要求，不是优化项）。
    pub fn ready_for_render(&self) -> Result<(), StkError> {
        self.validate()?;
        if self.storage.is_none() {
            return Err(
                StkError::simple(Domain::Render, 11, "缺少 storage size，禁止开始渲染")
                    .with_action(
                        "设置实际视频存储尺寸（ass_set_storage_size），不要复用 output canvas 顶替",
                    ),
            );
        }
        Ok(())
    }

    /// Preserve 模式：脚本坐标不改，缩放发生在 renderer 内部 → 返回 1。
    /// Rewrite 模式：canvas / script 的精确有理数比；两轴不一致即报错。
    pub fn script_scale_factor(&self) -> Result<Rational, StkError> {
        match self.script_policy {
            ScriptResolutionPolicy::Preserve => Ok(Rational::ONE),
            ScriptResolutionPolicy::Rewrite => {
                let s = self.script.ok_or_else(|| {
                    StkError::simple(Domain::Transform, 35, "Rewrite 模式需要已知 PlayRes")
                })?;
                let fx = Rational::new(i64::from(self.canvas.width), i64::from(s.x))
                    .map_err(|_| StkError::simple(Domain::Transform, 35, "PlayRes 缩放比非法"))?;
                let fy = Rational::new(i64::from(self.canvas.height), i64::from(s.y))
                    .map_err(|_| StkError::simple(Domain::Transform, 35, "PlayRes 缩放比非法"))?;
                if fx != fy {
                    return Err(StkError::simple(
                        Domain::Transform,
                        36,
                        "PlayRes 与目标画布比例不一致，拒绝各轴分别缩放",
                    )
                    .with_detail(format!("script {}x{} canvas {}", s.x, s.y, self.canvas)));
                }
                Ok(fx)
            }
        }
    }

    /// Rewrite 时 LayoutRes 的实际系数（09 §4 B：默认不跟 PlayRes 一起乘）。
    pub fn layout_scale_factor(&self) -> Result<Rational, StkError> {
        let base = self.script_scale_factor()?;
        match self.layout_policy {
            LayoutResPolicy::KeepAsIs => Ok(Rational::ONE),
            LayoutResPolicy::Drop => Ok(Rational::ONE),
            LayoutResPolicy::ScaleWithScript => {
                if self.layout.is_none() {
                    return Ok(Rational::ONE);
                }
                Ok(base)
            }
        }
    }

    /// 双重缩放预警：Rewrite + ScaleWithScript 时，libass 可能既按 LayoutRes 又按放大后的 PlayRes 计算。
    pub fn warns_double_scaling(&self) -> bool {
        self.script_policy == ScriptResolutionPolicy::Rewrite
            && self.layout_policy == LayoutResPolicy::ScaleWithScript
            && self.layout.is_some()
    }

    /// 存储尺寸 × PAR = 显示尺寸；用于校验 PAR 与画布是否自洽（非方形像素）。
    pub fn display_size_from_storage(&self) -> Result<(Rational, Rational), StkError> {
        let st = self
            .storage
            .ok_or_else(|| StkError::simple(Domain::Render, 11, "缺少 storage size"))?;
        let w = Rational::from_int(i64::from(st.width))
            .mul(&self.par.0)
            .ok_or_else(|| StkError::simple(Domain::Transform, 37, "PAR 计算溢出"))?;
        Ok((w, Rational::from_int(i64::from(st.height))))
    }

    /// 显示尺寸是否等价于目标画布（16:9 校验的 PAR 感知版本）。
    pub fn storage_matches_canvas(&self) -> Result<bool, StkError> {
        let (dw, dh) = self.display_size_from_storage()?;
        let lhs = dw
            .mul(&Rational::from_int(i64::from(self.canvas.height)))
            .ok_or_else(|| StkError::simple(Domain::Transform, 37, "比例校验溢出"))?;
        let rhs = dh
            .mul(&Rational::from_int(i64::from(self.canvas.width)))
            .ok_or_else(|| StkError::simple(Domain::Transform, 37, "比例校验溢出"))?;
        Ok(lhs == rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;

    fn g1080_to_2160(policy: ScriptResolutionPolicy) -> Geometry {
        Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            storage: Some(StorageSize::new(1920, 1080).unwrap()),
            script_policy: policy,
            ..Default::default()
        }
    }

    #[test]
    fn preserve_does_not_touch_script() {
        let g = g1080_to_2160(ScriptResolutionPolicy::Preserve);
        assert_eq!(g.script_scale_factor().unwrap(), Rational::ONE);
        assert_eq!(g.layout_scale_factor().unwrap(), Rational::ONE);
        // 09 §9：Preserve 不得改源脚本文本
        assert_eq!(g.script.unwrap().x, 1920);
    }

    #[test]
    fn rewrite_gives_exact_rational_two() {
        let g = g1080_to_2160(ScriptResolutionPolicy::Rewrite);
        assert_eq!(g.script_scale_factor().unwrap(), Rational::from_int(2));
    }

    #[test]
    fn rewrite_rejects_mismatched_aspect() {
        let mut g = g1080_to_2160(ScriptResolutionPolicy::Rewrite);
        g.script = Some(ScriptSize { x: 1920, y: 800 });
        let e = g.script_scale_factor().unwrap_err();
        assert_eq!(e.code_string(), "STK-TRF-0036");
    }

    #[test]
    fn rewrite_needs_known_playres() {
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Default::default()
        };
        assert!(g.script_scale_factor().is_err());
    }

    #[test]
    fn storage_is_mandatory_before_render() {
        // 40 §2.1：storage 不是可选优化
        let mut g = g1080_to_2160(ScriptResolutionPolicy::Preserve);
        assert!(g.ready_for_render().is_ok());
        g.storage = None;
        let e = g.ready_for_render().unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0011");
        assert!(StorageSize::new(0, 1080).is_err());
        assert!(Geometry::default().validate().is_ok(), "默认 1080p 合法");
    }

    #[test]
    fn non_16_9_needs_explicit_custom_flag() {
        let g = Geometry::for_canvas(Canvas {
            width: 1920,
            height: 800,
        });
        assert_eq!(g.validate().unwrap_err().code_string(), "STK-INP-0031");
        let mut c = g;
        c.canvas_is_custom = true;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn layout_keepasis_avoids_double_scale_warning() {
        let mut g = g1080_to_2160(ScriptResolutionPolicy::Rewrite);
        g.layout = Some(LayoutRes { x: 1920, y: 1080 });
        assert!(!g.warns_double_scaling(), "默认 KeepAsIs 不应报警");
        assert_eq!(g.layout_scale_factor().unwrap(), Rational::ONE);
        g.layout_policy = LayoutResPolicy::ScaleWithScript;
        assert!(g.warns_double_scaling());
        assert_eq!(g.layout_scale_factor().unwrap(), Rational::from_int(2));
    }

    #[test]
    fn par_makes_storage_match_wide_canvas() {
        // 1440x1080 存储 + PAR 4/3 = 1920x1080 显示（经典anamorphic 情形）
        let g = Geometry {
            canvas: Canvas::FHD1080,
            storage: Some(StorageSize::new(1440, 1080).unwrap()),
            par: PixelAspect::new(Rational::new(4, 3).unwrap()).unwrap(),
            ..Default::default()
        };
        assert!(g.storage_matches_canvas().unwrap());
        let square = Geometry {
            canvas: Canvas::FHD1080,
            storage: Some(StorageSize::new(1920, 1080).unwrap()),
            ..Default::default()
        };
        assert!(square.storage_matches_canvas().unwrap());
        // 该函数校验的是"存储尺寸×PAR 的显示比例是否与目标画布一致"，不是尺寸相等
        let same_ratio_other_size = Geometry {
            canvas: Canvas::FHD1080,
            storage: Some(StorageSize::new(1280, 720).unwrap()),
            ..Default::default()
        };
        assert!(same_ratio_other_size.storage_matches_canvas().unwrap());
        let wrong = Geometry {
            canvas: Canvas::FHD1080,
            storage: Some(StorageSize::new(1024, 768).unwrap()),
            ..Default::default()
        };
        assert!(!wrong.storage_matches_canvas().unwrap());
        assert!(PixelAspect::new(Rational::ZERO).is_err());
    }

    #[test]
    fn display_size_from_storage_is_rational() {
        let g = Geometry {
            canvas: Canvas::FHD1080,
            storage: Some(StorageSize::new(1440, 1080).unwrap()),
            par: PixelAspect::new(Rational::new(4, 3).unwrap()).unwrap(),
            ..Default::default()
        };
        let (w, h) = g.display_size_from_storage().unwrap();
        assert_eq!(w, Rational::from_int(1920));
        assert_eq!(h, Rational::from_int(1080));
    }
}
