use core_model::time::Rational;
use format_detect::SubtitleFormat;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct InputItem {
    pub id: usize,
    pub path: PathBuf,
    pub file_name: String,
    pub format: SubtitleFormat,
    pub selected: bool,
    pub duration_ms: Option<u64>,
    pub event_count: usize,
    pub play_res: Option<(u32, u32)>,
}

impl InputItem {
    pub fn from_path(id: usize, path: PathBuf) -> Self {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into());

        // 使用 format-detect 快速探测头部真实格式签名，不单纯相信扩展名
        let format = format_detect::probe_file(&path)
            .map(|d| d.format)
            .unwrap_or(SubtitleFormat::Unknown);

        Self {
            id,
            path,
            file_name,
            format,
            selected: true,
            duration_ms: None,
            event_count: 0,
            play_res: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VideoAssist {
    pub path: PathBuf,
    pub fps: Option<Rational>,
    pub resolution: Option<(u32, u32)>,
    pub is_hdr: bool,
    pub summary: String,
}

#[derive(Clone, Debug, Default)]
pub struct InputsState {
    pub items: Vec<InputItem>,
    pub active_preview_id: Option<usize>,
    pub video_assist: Option<VideoAssist>,
    pub banner_suggestion: Option<String>,
    next_id: usize,
}

impl InputsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_path(&mut self, path: PathBuf) {
        if self.items.iter().any(|item| item.path == path) {
            return; // 杜绝重复添加相同路径
        }
        let id = self.next_id;
        self.next_id += 1;
        let item = InputItem::from_path(id, path);
        if self.active_preview_id.is_none() {
            self.active_preview_id = Some(id);
        }
        self.items.push(item);
    }

    pub fn remove_item(&mut self, id: usize) {
        self.items.retain(|item| item.id != id);
        if self.active_preview_id == Some(id) {
            self.active_preview_id = self.items.first().map(|item| item.id);
        }
    }

    pub fn toggle_select(&mut self, id: usize) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.selected = !item.selected;
        }
    }

    pub fn select_all(&mut self, select: bool) {
        for item in &mut self.items {
            item.selected = select;
        }
    }

    pub fn active_input(&self) -> Option<&InputItem> {
        self.active_preview_id
            .and_then(|id| self.items.iter().find(|i| i.id == id))
    }

    pub fn selected_inputs(&self) -> Vec<&InputItem> {
        self.items.iter().filter(|i| i.selected).collect()
    }
}
