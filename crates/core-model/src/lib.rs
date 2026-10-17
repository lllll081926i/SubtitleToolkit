//! 基础模型层。ADR-007：核心不使用 float 表示时间/FPS；float 只在显示边界出现。

pub mod canvas;
pub mod color;
pub mod datadir;
pub mod error;
pub mod geometry;
pub mod loss;
pub mod profile;
pub mod settings;
pub mod time;

pub use canvas::Canvas;
pub use color::AssColor;
pub use datadir::{DataDir, DataDirKind};
pub use error::{Domain, Result, StkError};
pub use geometry::{
    Geometry, LayoutRes, LayoutResPolicy, PixelAspect, ScriptResolutionPolicy, ScriptSize,
    StorageSize,
};
pub use loss::LossReport;
pub use profile::{
    FormatKind, HdrMode, HdrSettings, JobSnapshot, OutputProfile, Plan, RendererChoice,
    ValidationLevel, ValueOrigin,
};
pub use settings::{AppSettings, SettingsLoad, SettingsSource, SettingsStore};
pub use time::{BdnTimecode, Fps, Rational, Time};
