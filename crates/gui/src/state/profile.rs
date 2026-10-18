use core_model::canvas::Canvas;
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
    /// 这个 profile 的组合有没有被硬规则挡住（AT-BD-001）。
    ///
    /// 返回 `Some(说明)` 表示**不该让它跑**，而说明是给用户看的原因。
    ///
    /// ## 为什么校验放在这里而不是各自的前端里
    ///
    /// CLI 侧同一条规则在 `core_model::profile::OutputProfile::validate`（`STK-VAL-0002`）。
    /// 两边各写一份的后果是**两边会逐渐不一致**：用户能在一个前端建一个
    /// 另一个前端拒绝的 profile，而两个都声称自己对。
    ///
    /// 所以这里**不重新实现判定**，只调用 `Canvas::within_bluray` 这个
    /// 组件级单一事实源（09 §6 的硬规则），把结论翻译成 GUI 的说明文案。
    pub fn problem(&self) -> Option<String> {
        // 严格蓝光 profile 禁止 2160p PGS（09 §6、AT-BD-001）。
        //
        // 判据依 `format` 与画布两个字段：PGS 是有蓝光规范的位图格式（
        // 规范的 PGS 就是蓝光字幕流），而 BDN XML + PNG 是同一族交换格式，
        // 蓝光同样不允许 2160p —— 所以两者适用同一条限制。
        // ASS/SRT 是文本格式，没有蓝光的画布上限。
        let canvas = match self.canvas {
            CanvasMode::Uhd2160 => Canvas::UHD2160,
            CanvasMode::Fhd1080 => Canvas::FHD1080,
            CanvasMode::Hd720 => Canvas::HD720,
            // 保持原尺寸与自定义：判定要等真实脚本/取值，这里不猜。
            CanvasMode::KeepOriginal | CanvasMode::Custom(..) => return None,
        };
        let is_bluray_bitmap =
            matches!(self.target_format, TargetFormat::Pgs | TargetFormat::BdnXml);
        if is_bluray_bitmap && !canvas.within_bluray() {
            return Some(format!(
                "{} 不允许 {} 画布：蓝光规范的位图字幕最大是 1920x1080。\n\n\
                 这一条是硬规则（09 §6），改了画布就不合规。\n\
                 可选的改法：\n\
                 · 画布改成 1920x1080（保持同一种输出格式）；\n\
                 · 或者保留 4K 而把输出格式改成 ASS —— 文本字幕没有蓝光的画布上限。",
                self.target_format.label(),
                canvas.label()
            ));
        }
        None
    }

    /// 只想问“能不能跑”，不要说明。
    pub fn is_runnable(&self) -> bool {
        self.problem().is_none()
    }

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

    /// 出厂默认的第二个 profile 是 **1080p HDR PGS**，不是 2160p。
    ///
    /// 历史上这里给的是 2160p，而那是一个**自己后端都拒收**的默认值：
    /// `pgs encode` 默认 `--level standard`，而标准级 PGS 不允许 3840x2160
    /// —— 实测报 `STK-PGS-0091: 画布超出 Blu-ray PGS profile 上限 1920x1080`。
    ///
    /// 一个出厂默认 profile 在用户什么都没改的情况下就失败，是最坏的一种默认：
    /// 用户会以为程序坏了，而不是以为自己选错了。
    ///
    /// HDR 映射照旧给 ReferenceWhite(203)（这是有意选的一个有意义的默认，
    /// 不是 2160p 的副产品）。想要 4K 的用户可以自己把画布改成 3840x2160，
    /// 那时 `problem()` 会明确告诉他为什么不行、以及该怎么改。
    pub fn default_hdr_sup(id: usize) -> Self {
        Self {
            id,
            enabled: true,
            name: "1080p HDR SUP".into(),
            target_format: TargetFormat::Pgs,
            canvas: CanvasMode::Fhd1080,
            time_adjust: TimeAdjustMode::None,
            hdr: HdrMode::ReferenceWhite(203),
            custom_suffix: ".hdr.sup".into(),
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
        let p2 = OutputProfile::default_hdr_sup(2);
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

#[cfg(test)]
mod tests {
    use super::*;

    // AT-BD-001：严格蓝光 profile 禁止 2160p PGS；UI 必须明确解释。
    // 这里核的是**解释存在且能定位到原因**，不是“有个字符串”。

    #[test]
    fn at_bd_001_2160p_pgs_is_rejected_with_an_explanation() {
        let mut p = OutputProfile::default_hdr_sup(2);
        assert_eq!(p.target_format, TargetFormat::Pgs);
        // 默认是 1080p，所以这里主动改成 4K 来触发拒绝。
        p.canvas = CanvasMode::Uhd2160;
        assert!(!p.is_runnable());

        let why = p.problem().expect("2160p PGS 应当被拒");
        // 说明里要能看出三件事：哪个格式、实际画布、正确尺寸。
        assert!(why.contains("PGS"), "说明没提格式: {why}");
        assert!(why.contains("3840x2160"), "说明没提实际画布: {why}");
        assert!(why.contains("1920x1080"), "说明没给可用的改法: {why}");
    }

    #[test]
    fn at_bd_001_2160p_bdn_is_rejected() {
        // BDN XML + PNG 是同一族的蓝光位图交换格式，适用同一条限制。
        let mut p = OutputProfile::default_bdn(1);
        p.canvas = CanvasMode::Uhd2160;
        assert_eq!(p.target_format, TargetFormat::BdnXml);
        assert!(p.problem().is_some());
    }

    #[test]
    fn at_bd_001_2160p_ass_is_allowed() {
        // 文本格式没有蓝光的画布上限：4K 是合理的 ASS 输出。
        let mut p = OutputProfile::default_bdn(1);
        p.target_format = TargetFormat::Ass;
        p.canvas = CanvasMode::Uhd2160;
        assert!(
            p.is_runnable(),
            "ASS 在 4K 下应当可跑，而 problem() 说: {:?}",
            p.problem()
        );
    }

    #[test]
    fn at_bd_001_1080p_pgs_is_allowed() {
        // 边界：1920x1080 恰好是上限，必须放行。
        let p = OutputProfile::default_hdr_sup(2);
        assert_eq!(p.canvas, CanvasMode::Fhd1080);
        assert!(p.is_runnable(), "1080p PGS 应当可跑");
    }

    #[test]
    fn at_bd_001_720p_bitmap_is_allowed() {
        // 上限的另一侧也要放行。
        let mut p = OutputProfile::default_bdn(1);
        p.canvas = CanvasMode::Hd720;
        assert!(p.is_runnable(), "720p BDN 应当可跑");
    }

    #[test]
    fn at_bd_001_keep_original_does_not_pretend_to_know() {
        // “保持原尺寸”要到真脚本才知道尺寸，这里不能猜。
        // 无声放行是对的：猜错会把能跑的活儿拦下来。
        let mut p = OutputProfile::default_hdr_sup(2);
        p.canvas = CanvasMode::KeepOriginal;
        assert!(p.is_runnable());
    }

    #[test]
    fn the_default_profiles_are_all_runnable() {
        // 出厂的三个默认 profile 不能有一个跑不了：
        // 拼错默认值会让用户开箱即撞上一个自己没设过的错。
        for p in ProfilesState::default().profiles {
            assert!(
                p.is_runnable(),
                "默认 profile「{}」就是不合规的: {:?}",
                p.name,
                p.problem()
            );
        }
    }

    #[test]
    fn the_default_sup_profile_is_not_4k() {
        // 回归用例：出厂默认的那个 PGS profile 曾经是 2160p，
        // 而 `pgs encode` 默认 `--level standard`，标准级 PGS 不允许 3840x2160
        // —— 后端报 `STK-PGS-0091`。即“出厂默认自己跑不起来”。
        //
        // 现在钉住：默认 PGS profile 的画布必须在蓝光上限内，且 HDR 映射还在。
        let p = OutputProfile::default_hdr_sup(2);
        assert_eq!(p.target_format, TargetFormat::Pgs);
        assert_eq!(p.canvas, CanvasMode::Fhd1080);
        assert!(p.is_runnable(), "默认 PGS profile 必须能直接跑");
        assert_eq!(p.hdr, HdrMode::ReferenceWhite(203));
    }

    #[test]
    fn gui_agrees_with_the_core_rule_it_reuses() {
        // GUI 与 CLI 必须对同一个组合给出同一结论。
        // 这里直接拿 core_model 的判定当参照：GUI 不该多一套自己的规则。
        for canvas in [Canvas::HD720, Canvas::FHD1080, Canvas::UHD2160] {
            let mode = match canvas {
                Canvas::HD720 => CanvasMode::Hd720,
                Canvas::FHD1080 => CanvasMode::Fhd1080,
                _ => CanvasMode::Uhd2160,
            };
            for format in [TargetFormat::Pgs, TargetFormat::BdnXml] {
                let mut p = OutputProfile::default_bdn(1);
                p.target_format = format;
                p.canvas = mode;
                assert_eq!(
                    p.is_runnable(),
                    canvas.within_bluray(),
                    "{format:?} + {canvas} 上 GUI 与 core 的判定不一致"
                );
            }
        }
    }
}
