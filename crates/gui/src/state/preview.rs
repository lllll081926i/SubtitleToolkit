#[derive(Clone, Debug)]
pub struct PreviewState {
    pub current_time_ms: i64,
    pub duration_ms: i64,
    pub is_split_ab: bool,
    pub zoom: f32,
    pub rendered_rgba: Option<Vec<u8>>,
    pub rendered_dims: (u32, u32),
    pub font_health_summary: Vec<String>,
    pub is_rendering: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            current_time_ms: 0,
            duration_ms: 60_000, // 默认 1 分钟区间，等待输入文件加载后更新
            is_split_ab: false,
            zoom: 1.0,
            rendered_rgba: None,
            rendered_dims: (1920, 1080),
            font_health_summary: Vec::new(),
            is_rendering: false,
        }
    }
}

impl PreviewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_time(&mut self, time_ms: i64) {
        self.current_time_ms = time_ms.clamp(0, self.duration_ms);
    }

    pub fn toggle_split_ab(&mut self) {
        self.is_split_ab = !self.is_split_ab;
    }

    pub fn set_frame(&mut self, rgba: Vec<u8>, width: u32, height: u32) {
        self.rendered_rgba = Some(rgba);
        self.rendered_dims = (width, height);
        self.is_rendering = false;
    }
}
