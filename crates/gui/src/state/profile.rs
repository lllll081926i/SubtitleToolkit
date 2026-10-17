use core_model::time::Rational;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetFormat {
    BdnXml,
    Pgs,
    Ass,
    Srt,
}

impl TargetFormat {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BdnXml => "BDN XML + PNG",
            Self::Pgs => "PGS (.sup)",
            Self::Ass => "ASS 特效字幕",
            Self::Srt => "纯文本 SRT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasMode {
    KeepOriginal,
    Fhd1080,
    Uhd2160,
    Hd720,
    Custom(u32, u32),
}

impl CanvasMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::KeepOriginal => "保持脚本原始尺寸",
            Self::Fhd1080 => "1080p (1920x1080)",
            Self::Uhd2160 => "4K UHD (3840x2160)",
            Self::Hd720 => "720p (1280x720)",
            Self::Custom(..) => "自定义 16:9 画布",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeAdjustMode {
    None,
    SpeedScale { from: Rational, to: Rational },
    OffsetMs(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrMode {
    Sdr,
    Percentage(u32),     // 百分比，例如 50%
    ReferenceWhite(u32), // 参考白 nit，例如 203
}

#[derive(Clone, Debug)]
pub struct OutputProfile {
    pub id: usize,
    pub enabled: bool,
    pub name: String,
    pub target_format: TargetFormat,
    pub canvas: CanvasMode,
    pub time_adjust: TimeAdjustMode,
    pub hdr: HdrMode,
    pub custom_suffix: String,
}

impl OutputProfile {
    pub fn default_bdn(id: usize) -> Self {
        Self {
            id,
            enabled: true,
            name: "1080p Blu-ray BDN".into(),
            target_format: TargetFormat::BdnXml,
            canvas: CanvasMode::Fhd1080,
            time_adjust: TimeAdjustMode::None,
            hdr: HdrMode::Sdr,
            custom_suffix: ".bdn".into(),
        }
    }

    pub fn default_uhd_sup(id: usize) -> Self {
        Self {
            id,
            enabled: true,
            name: "2160p 4K HDR SUP".into(),
            target_format: TargetFormat::Pgs,
            canvas: CanvasMode::Uhd2160,
            time_adjust: TimeAdjustMode::None,
            hdr: HdrMode::ReferenceWhite(203),
            custom_suffix: ".4k.sup".into(),
        }
    }

    pub fn default_srt(id: usize) -> Self {
        Self {
            id,
            enabled: false,
            name: "纯文本 SRT 导出".into(),
            target_format: TargetFormat::Srt,
            canvas: CanvasMode::KeepOriginal,
            time_adjust: TimeAdjustMode::None,
            hdr: HdrMode::Sdr,
            custom_suffix: ".srt".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProfilesState {
    pub profiles: Vec<OutputProfile>,
    pub active_edit_id: Option<usize>,
    next_id: usize,
}

impl Default for ProfilesState {
    fn default() -> Self {
        let p1 = OutputProfile::default_bdn(1);
        let p2 = OutputProfile::default_uhd_sup(2);
        let p3 = OutputProfile::default_srt(3);
        Self {
            profiles: vec![p1, p2, p3],
            active_edit_id: Some(1),
            next_id: 4,
        }
    }
}

impl ProfilesState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_profile(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let mut p = OutputProfile::default_bdn(id);
        p.name = format!("新 Profile {}", id);
        self.active_edit_id = Some(id);
        self.profiles.push(p);
    }

    pub fn duplicate_profile(&mut self, source_id: usize) {
        if let Some(src) = self.profiles.iter().find(|p| p.id == source_id).cloned() {
            let id = self.next_id;
            self.next_id += 1;
            let mut dup = src;
            dup.id = id;
            dup.name = format!("{} (副本)", dup.name);
            self.profiles.push(dup);
            self.active_edit_id = Some(id);
        }
    }

    pub fn remove_profile(&mut self, id: usize) {
        if self.profiles.len() > 1 {
            self.profiles.retain(|p| p.id != id);
            if self.active_edit_id == Some(id) {
                self.active_edit_id = self.profiles.first().map(|p| p.id);
            }
        }
    }

    pub fn active_profile_mut(&mut self) -> Option<&mut OutputProfile> {
        let active = self.active_edit_id;
        self.profiles.iter_mut().find(|p| Some(p.id) == active)
    }

    pub fn enabled_profiles(&self) -> Vec<&OutputProfile> {
        self.profiles.iter().filter(|p| p.enabled).collect()
    }

    /// 当前选中的（正在编辑的）profile。
    ///
    /// 回退顺序：`active_edit_id` → 第一个启用的 → 第一个。
    /// 这个回退是有意的：A/B 对比要展示“当前 Profile 的变换”，
    /// 而“当前”在用户没点过任何 profile 时应当是那个**会真的拿去做导出**的。
    pub fn active_profile(&self) -> Option<&OutputProfile> {
        self.active_edit_id
            .and_then(|id| self.profiles.iter().find(|p| p.id == id))
            .or_else(|| self.profiles.iter().find(|p| p.enabled))
            .or_else(|| self.profiles.first())
    }
}
