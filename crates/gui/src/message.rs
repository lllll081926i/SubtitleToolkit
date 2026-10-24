use crate::state::profile::{CanvasMode, HdrMode, TargetFormat, TimeAdjustMode};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Message {
    // === 左栏：输入管理 ===
    InputSelected(usize),
    InputToggled(usize),
    SelectAllInputs(bool),
    RemoveInput(usize),
    AddFilesClicked,
    FilesDropped(Vec<PathBuf>),
    VideoAssistDropped(PathBuf),
    ApplyVideoAssist,
    DismissVideoBanner,

    // === 中栏：预览与时间定位 ===
    PreviewTimeChanged(i64),
    PreviewJumpEvent(bool), // true = 下一句，false = 上一句
    ToggleSplitAb,
    FrameRendered {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        /// 这一帧是怎么来的（真渲染 / 占位色块 / 失败）。
        /// 不能丢：丢了之后界面会把占位色块当成真实预览。
        source: crate::state::preview::FrameSource,
    },
    /// A/B 两侧都渲染好了（05 §4.1）。
    ///
    /// 两条分开传，而不是各发一条 `FrameRendered`：
    /// 那种写法会在两条消息之间出现“新旧混搭”的中间状态（左半是这一刻的、
    /// 右半是上一刻的），而那种不一致看起来和真实的颜色差异一模一样。
    AbRendered {
        a_rgba: Vec<u8>,
        a_dims: (u32, u32),
        b_rgba: Vec<u8>,
        b_dims: (u32, u32),
        note: String,
        source: crate::state::preview::FrameSource,
    },

    // === 右栏：Profile 矩阵 ===
    ProfileSelected(usize),
    ProfileToggled(usize),
    AddProfile,
    DuplicateProfile(usize),
    RemoveProfile(usize),
    FormatChanged(usize, TargetFormat),
    CanvasChanged(usize, CanvasMode),
    HdrChanged(usize, HdrMode),
    TimeAdjustChanged(usize, TimeAdjustMode),
    ExportPreset,
    ImportPreset,
    OpenSettings,
    CloseSettings,
    SettingLogLevelChanged(String),
    SettingDefaultFpsChanged(String),
    SettingDefaultCanvasChanged(String),
    SaveSettings,

    // === 底栏：任务抽屉与执行 ===
    StartQueue,
    CancelQueue,
    /// AT-RENDER-002 的“GUI 可重试”：**只重跑失败与没跑到的那些格**。
    ///
    /// 与 `StartQueue` 分开而不是复用它：那个是“把整批重来”，
    /// 会重写已经好的产物（见 `TaskDrawerState::retry_targets` 的说明）。
    RetryFailed,
    ToggleTaskDrawer,
    SelectOutputDir,
    OpenOutputDir,
    /// Iced Task::run 流式发送阶段进度，不再等整个批次结束。
    BatchEvent(crate::worker::task_worker::Progress),
    BatchCompleted(Option<(String, String, String)>),
    /// 编译期“做不到”的说明（时间轴/HDR 在 batch spec 里没有落点）。
    TaskNotes(Vec<String>),

    // === 顶部栏 ===
    OpenDoctorDialog,
    CloseDialog,
    /// 环境自检完成：带上真实 doctor 报告（或拿不到时的说明）。
    ///
    /// 之前弹窗只有一个bool，点了什么都不会显示。"按钮在但没反应"
    /// 与"功能没做"在用户看来是同一件事，而前者更难查。
    DoctorLoaded(Box<Result<serde_json::Value, String>>),
}
