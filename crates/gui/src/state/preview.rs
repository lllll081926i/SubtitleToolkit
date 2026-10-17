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
    /// 当前帧是怎么来的。
    ///
    /// `Placeholder` 不是可有可无的装饰：预览拿不到 libass 时会画一个合成色块，
    /// 不标出来的话用户会把它当成"我的字幕就长这样"——那是错的，
    /// 且它的颜色、位置、字体全是我们编的。
    pub source: FrameSource,
}

/// 一帧的来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameSource {
    /// 还没渲染过
    None,
    /// 真 libass 经 renderer-host 出的像素
    Renderer { provider: String },
    /// 占位色块（拿不到 libass / renderer-host），带原因
    Placeholder { reason: String },
    /// 渲染失败，带错误码
    Failed { code: String },
}

impl FrameSource {
    pub fn label(&self) -> String {
        match self {
            FrameSource::None => "尚未渲染".to_string(),
            FrameSource::Renderer { provider } => format!("真实渲染（{provider}）"),
            FrameSource::Placeholder { reason } => format!("⚠ 占位色块，不是真实效果：{reason}"),
            FrameSource::Failed { code } => format!("渲染失败（{code}）"),
        }
    }

    /// 当前画面是否可用于判断字幕外观。占位与失败都**不能**。
    pub fn is_trustworthy(&self) -> bool {
        matches!(self, FrameSource::Renderer { .. })
    }
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
            source: FrameSource::None,
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
        // 不知道来源就不冒充真实渲染：调用方必须显式说清楚它给的是什么。
        self.source = FrameSource::Placeholder {
            reason: "调用方未标明像素来源".to_string(),
        };
    }

    /// 真渲染结果。
    pub fn set_rendered_frame(
        &mut self,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        provider: impl Into<String>,
    ) {
        self.set_frame(rgba, width, height);
        self.source = FrameSource::Renderer {
            provider: provider.into(),
        };
    }

    /// 占位色块（带原因）。
    pub fn set_placeholder_frame(
        &mut self,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        reason: impl Into<String>,
    ) {
        self.set_frame(rgba, width, height);
        self.source = FrameSource::Placeholder {
            reason: reason.into(),
        };
    }

    pub fn clear_frame(&mut self, code: impl Into<String>) {
        self.rendered_rgba = None;
        self.is_rendering = false;
        self.source = FrameSource::Failed { code: code.into() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 占位帧绝不能被当成真实预览。
    ///
    /// 这是本模块存在的理由：拿不到 libass 时会画一个合成色块，
    /// 而它的颜色、位置、字体全是我们编的。不标出来，用户就会以为
    /// "我的字幕就长这样"——那是错的。
    #[test]
    fn a_placeholder_frame_is_never_trustworthy() {
        let mut p = PreviewState::new();
        assert_eq!(p.source, FrameSource::None);
        assert!(!p.source.is_trustworthy(), "还没渲染过就不该可信");

        p.set_placeholder_frame(vec![0u8; 4], 1, 1, "找不到 libass");
        assert!(!p.source.is_trustworthy(), "占位色块不是真实效果");
        let label = p.source.label();
        assert!(label.contains("占位"), "{label}");
        assert!(label.contains("找不到 libass"), "原因必须可见：{label}");
    }

    #[test]
    fn a_real_render_is_trustworthy_and_names_the_provider() {
        let mut p = PreviewState::new();
        p.set_rendered_frame(vec![1, 2, 3, 4], 1, 1, "libass-9.dll");
        assert!(p.source.is_trustworthy());
        assert!(
            p.source.label().contains("libass-9.dll"),
            "{}",
            p.source.label()
        );
    }

    /// 渲染失败时必须清掉旧帧。
    ///
    /// 留着上一帧会让人以为"这就是现在这一刻的样子"，而那是上一刻的像素。
    #[test]
    fn a_failed_render_clears_the_previous_frame() {
        let mut p = PreviewState::new();
        p.set_rendered_frame(vec![1, 2, 3, 4], 1, 1, "libass");
        assert!(p.rendered_rgba.is_some());
        p.clear_frame("STK-RND-0001");
        assert!(p.rendered_rgba.is_none(), "旧像素必须被清掉");
        assert!(!p.is_rendering);
        assert!(
            matches!(p.source, FrameSource::Failed { .. }),
            "{:?}",
            p.source
        );
        assert!(p.source.label().contains("STK-RND-0001"));
        assert!(!p.source.is_trustworthy());
    }

    /// `set_frame` 不知道来源，就不能冒充真实渲染。
    #[test]
    fn the_generic_setter_does_not_claim_to_be_a_real_render() {
        let mut p = PreviewState::new();
        p.set_frame(vec![0u8; 4], 1, 1);
        assert!(!p.source.is_trustworthy(), "{:?}", p.source);
        assert!(matches!(p.source, FrameSource::Placeholder { .. }));
    }
}
