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
    pub event_starts: Vec<i64>,
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

        let mut item = Self {
            id,
            path,
            file_name,
            format,
            selected: true,
            duration_ms: None,
            event_count: 0,
            event_starts: Vec::new(),
            play_res: None,
        };
        item.load_metadata();
        item
    }
    fn load_metadata(&mut self) {
        if !matches!(
            self.format,
            SubtitleFormat::Ass | SubtitleFormat::Ssa | SubtitleFormat::Srt
        ) {
            return;
        }
        let Ok(loaded) = cli::pipeline::load_text(&self.path) else {
            return;
        };
        let pairs: Vec<_> = if self.format == SubtitleFormat::Srt {
            let Ok((sub, _)) = codec_srt::parse(loaded.text.as_bytes()) else {
                return;
            };
            sub.cues.iter().map(|cue| (cue.start, cue.end)).collect()
        } else {
            let doc = codec_ass::AssDocument::parse(&loaded.text);
            self.play_res = doc
                .play_res()
                .and_then(|(w, h)| Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?)))
                .filter(|(w, h)| core_model::Canvas::new(*w, *h).is_ok());
            doc.dialogues()
                .iter()
                .filter_map(|d| Some((d.start().ok()?, d.end().ok()?)))
                .collect()
        };
        self.event_count = pairs.len();
        self.event_starts = pairs
            .iter()
            .filter_map(|(start, _)| start.to_pts_ticks().ok().map(|t| (t / 90).max(0)))
            .collect();
        self.event_starts.sort_unstable();
        self.event_starts.dedup();
        self.duration_ms = pairs
            .iter()
            .filter_map(|(_, end)| end.to_pts_ticks().ok().map(|t| (t / 90).max(0) as u64))
            .max();
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
        self.add_item(InputItem::from_path(0, path));
    }

    pub fn add_item(&mut self, mut item: InputItem) {
        if self.items.iter().any(|old| old.path == item.path) {
            return;
        }
        item.id = self.next_id;
        self.next_id += 1;
        if self.active_preview_id.is_none() {
            self.active_preview_id = Some(item.id);
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
