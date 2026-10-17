//! 基础模型层。ADR-007：核心不使用 float 表示时间/FPS；float 只在显示边界出现。

pub mod canvas;
pub mod color;
pub mod datadir;
pub mod error;
pub mod geometry;
pub mod logging;
pub mod loss;
pub mod pe;
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
pub use logging::{current_log_file, init as init_logging, log_dir};
pub use loss::LossReport;
// PE 头读取（23 §DLL / 12 §3）：放在 core-model 是因为主程序与 renderer-host
// 都要在加载 DLL 之前判定位数，而**两份实现对同一个 DLL 会给出不同答案**。
pub use pe::{arch_of_bytes, arch_of_file, PeArch, PeError};
pub use profile::{
    FormatKind, HdrMode, HdrSettings, JobSnapshot, OutputProfile, Plan, RendererChoice,
    ValidationLevel, ValueOrigin,
};
pub use settings::{AppSettings, SettingsLoad, SettingsSource, SettingsStore};
pub use time::{BdnTimecode, Fps, Rational, Time};
