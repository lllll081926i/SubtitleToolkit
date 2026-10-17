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

    // === 底栏：任务抽屉与执行 ===
    StartQueue,
    CancelQueue,
    ToggleTaskDrawer,
    SelectOutputDir,
    OpenOutputDir,
    TaskProgress {
        id: usize,
        progress: f32,
        message: String,
    },
    TaskFinished {
        id: usize,
        result: Result<(), (String, String, String)>,
    },

    // === 顶部栏 ===
    OpenDoctorDialog,
    CloseDialog,
}
