use crate::state::profile::{
    CanvasMode, HdrMode, OutputProfile, ProfilesState, TargetFormat, TimeAdjustMode,
};
use core_model::preset::{HdrTarget, OutputTarget, Preset};
use std::collections::BTreeMap;

pub fn from_profiles(state: &ProfilesState) -> Result<Preset, String> {
    let mut outputs = Vec::new();
    for profile in state.enabled_profiles() {
        if profile.time_adjust != TimeAdjustMode::None {
            return Err(format!(
                "Profile「{}」含时间变换；当前 preset schema 和 GUI 批处理不能保存该参数。请先将时间变换设为无，再导出。",
                profile.name
            ));
        }
        let format = match profile.target_format {
            TargetFormat::BdnXml => "bdn",
            TargetFormat::Pgs => "pgs",
            TargetFormat::Ass => "ass",
            TargetFormat::Srt => "srt",
        };
        let canvas = match profile.canvas {
            CanvasMode::KeepOriginal => None,
            CanvasMode::Fhd1080 => Some([1920, 1080]),
            CanvasMode::Uhd2160 => Some([3840, 2160]),
            CanvasMode::Hd720 => Some([1280, 720]),
            CanvasMode::Custom(w, h) => Some([w, h]),
        };
        let hdr_mode = match profile.hdr {
            HdrMode::Sdr => "off".to_string(),
            HdrMode::ReferenceWhite(n) => format!("reference-white:{n}"),
            HdrMode::Percentage(p) => format!("scale:{p}"),
        };
        outputs.push(OutputTarget {
            name: profile.name.clone(),
            format: Some(format.to_string()),
            canvas,
            fps: None,
            hdr: Some(HdrTarget {
                mode: Some(hdr_mode),
            }),
            suffix: Some(profile.custom_suffix.clone()),
            renderer: None,
            pgs: None,
        });
    }
    if outputs.is_empty() {
        return Err("没有启用的输出 Profile，无法导出 preset。".to_string());
    }
    Ok(Preset {
        schema: core_model::preset::PRESET_CURRENT_SCHEMA,
        name: Some("SubtitleToolkit GUI 输出配置".to_string()),
        outputs,
        unknown: BTreeMap::new(),
    })
}

pub fn into_profiles(preset: &Preset) -> Result<(Vec<OutputProfile>, Vec<String>), String> {
    let mut profiles = Vec::with_capacity(preset.outputs.len());
    let mut notices = Vec::new();
    for (index, output) in preset.outputs.iter().enumerate() {
        let target_format = match output.format.as_deref().unwrap_or("bdn") {
            "bdn" | "xml" => TargetFormat::BdnXml,
            "pgs" | "sup" => TargetFormat::Pgs,
            "ass" | "ssa" => TargetFormat::Ass,
            "srt" => TargetFormat::Srt,
            other => {
                return Err(format!(
                    "preset 输出「{}」使用 GUI 不支持的格式 {other}。",
                    output.name
                ))
            }
        };
        let canvas = match output.canvas {
            None => CanvasMode::KeepOriginal,
            Some([1920, 1080]) => CanvasMode::Fhd1080,
            Some([3840, 2160]) => CanvasMode::Uhd2160,
            Some([1280, 720]) => CanvasMode::Hd720,
            Some([w, h]) => CanvasMode::Custom(w, h),
        };
        let hdr = match output.hdr.as_ref().and_then(|hdr| hdr.mode.as_deref()) {
            None | Some("off") => HdrMode::Sdr,
            Some(mode) if mode.starts_with("reference-white:") => mode
                .trim_start_matches("reference-white:")
                .parse::<u32>()
                .map(HdrMode::ReferenceWhite)
                .map_err(|_| format!("preset 输出「{}」的 HDR 参考白值无效。", output.name))?,
            Some(mode) if mode.starts_with("scale:") => mode
                .trim_start_matches("scale:")
                .parse::<u32>()
                .map(HdrMode::Percentage)
                .map_err(|_| format!("preset 输出「{}」的 HDR 缩放值无效。", output.name))?,
            Some(mode) => {
                return Err(format!(
                    "preset 输出「{}」的 HDR 模式 {mode} 暂不支持。",
                    output.name
                ))
            }
        };
        if output.fps.is_some() || output.renderer.is_some() || output.pgs.is_some() {
            notices.push(format!(
                "「{}」中的帧率、渲染器或 PGS 校验参数未导入：当前 GUI 尚无对应控制项。",
                output.name
            ));
        }
        profiles.push(OutputProfile {
            id: index + 1,
            enabled: true,
            name: output.name.clone(),
            target_format,
            canvas,
            time_adjust: TimeAdjustMode::None,
            hdr,
            custom_suffix: output.suffix.clone().unwrap_or_default(),
        });
    }
    Ok((profiles, notices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_profiles_roundtrip_through_the_real_preset_schema() {
        let state = ProfilesState::default();
        let preset = from_profiles(&state).unwrap();
        let encoded = core_model::preset::to_toml(&preset).unwrap();
        let (decoded, problems) = core_model::preset::parse(&encoded).unwrap();
        assert!(problems.is_empty());
        let (profiles, notices) = into_profiles(&decoded).unwrap();
        assert!(notices.is_empty());
        assert_eq!(profiles.len(), 2, "只导出启用的 Profile");
        assert_eq!(profiles[0].target_format, TargetFormat::BdnXml);
        assert_eq!(profiles[1].target_format, TargetFormat::Pgs);
        assert_eq!(profiles[1].hdr, HdrMode::ReferenceWhite(203));
    }

    #[test]
    fn preset_fields_without_gui_controls_are_reported_on_import() {
        let text = r#"
schema = 1
[[outputs]]
name = "影视 ASS"
format = "ass"
fps = "24000/1001"
renderer = "libass"
"#;
        let (preset, _) = core_model::preset::parse(text).unwrap();
        let (_, notices) = into_profiles(&preset).unwrap();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("帧率、渲染器"));
    }

    #[test]
    fn export_refuses_to_drop_profile_time_adjustment() {
        let mut state = ProfilesState::default();
        state.profiles[0].time_adjust = TimeAdjustMode::OffsetMs(120);
        assert!(from_profiles(&state).unwrap_err().contains("时间变换"));
    }
}
