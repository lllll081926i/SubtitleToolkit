//! Provider 能力探测（12 §1B/C、§3、§4；40 §9）。
//! 关键规则：绝不靠 DLL 文件名推断调用 ABI，必须按 PE arch → 导出 API → CSRI capability → legacy capability 逐级探测。

use core_model::error::{Domain, Result, StkError};

use crate::Arch;

/// 三条互相独立的能力链（12 §1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProviderKind {
    /// 内置 libass
    LibassBuiltin,
    /// 标准 CSRI provider（csri_stream_ext + BGRA）
    Csri,
    /// VapourSynth 接口（vsfm.TextSubMod 等）
    LegacyVapourSynth,
    /// AviSynth+ 接口（TextSub/MaskSub）
    LegacyAviSynth,
    /// DirectShow/VFW 等其它旧路径，需要额外适配
    LegacyOther,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeStage {
    PeArch,
    ExportedApi,
    CsriCapability,
    LegacyCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub stages: Vec<(ProbeStage, Outcome, String)>,
    pub kind: Option<ProviderKind>,
}

impl ProbeResult {
    pub fn stage(&self, s: ProbeStage) -> Option<Outcome> {
        self.stages
            .iter()
            .find(|(k, _, _)| *k == s)
            .map(|(_, o, _)| *o)
    }
}

/// CSRI 必需导出名集合。最终名单待与 csri.h 核对（见 docs/ISSUES.md C9），
/// 这里按 CSRI C API 的公开符号取保守集合，缺任何一个即判定"不是 CSRI provider"。
pub const CSRI_REQUIRED_EXPORTS: [&str; 4] = [
    "csri_open",
    "csri_close",
    "csri_render",
    "csri_get_renderer",
];

/// 探测主流程。`exports` 由 PE 解析层提供（不在本 crate 里读 DLL，保持可测试）。
pub fn probe(filename_hint: &str, exports: &[String]) -> ProbeResult {
    let mut stages: Vec<(ProbeStage, Outcome, String)> = Vec::new();
    let missing: Vec<&str> = CSRI_REQUIRED_EXPORTS
        .iter()
        .copied()
        .filter(|n| !exports.iter().any(|e| e.as_str() == *n))
        .collect();
    let has_csri = missing.is_empty();
    stages.push((
        ProbeStage::ExportedApi,
        if exports.is_empty() {
            Outcome::Fail
        } else {
            Outcome::Pass
        },
        format!("导出符号 {} 个", exports.len()),
    ));
    stages.push((
        ProbeStage::CsriCapability,
        if has_csri {
            Outcome::Pass
        } else {
            Outcome::Fail
        },
        if has_csri {
            "标准 CSRI 符号齐全".to_string()
        } else {
            format!("缺少 CSRI 符号: {missing:?}")
        },
    ));
    let legacy = if has_csri {
        Outcome::NotApplicable
    } else {
        let vs = exports
            .iter()
            .any(|e| e.starts_with("getVapourSynthAPI") || e.starts_with("vsfm"));
        let avs = exports
            .iter()
            .any(|e| e.starts_with("avs_") || e.contains("CreateAudioRenderer"));
        if vs || avs {
            Outcome::Pass
        } else {
            Outcome::Fail
        }
    };
    stages.push((
        ProbeStage::LegacyCapability,
        legacy,
        format!("文件名提示: {filename_hint}"),
    ));

    let kind = if has_csri {
        Some(ProviderKind::Csri)
    } else if exports
        .iter()
        .any(|e| e.starts_with("getVapourSynthAPI") || e.starts_with("vsfm"))
    {
        Some(ProviderKind::LegacyVapourSynth)
    } else if exports.iter().any(|e| e.starts_with("avs_")) {
        Some(ProviderKind::LegacyAviSynth)
    } else {
        None
    };
    ProbeResult { stages, kind }
}

/// 从 PE 文件头读 Machine 字段（12 §3：不得跨位数 LoadLibrary）。
pub fn pe_arch(bytes: &[u8]) -> Result<Arch> {
    let at = |off: usize, len: usize| -> Result<&[u8]> {
        bytes
            .get(off..off + len)
            .ok_or_else(|| StkError::simple(Domain::Filter, 40, "PE 头被截断"))
    };
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return Err(StkError::simple(
            Domain::Filter,
            41,
            "不是有效的 PE 文件（缺 MZ 签名）",
        ));
    }
    let lfanew = u32::from_le_bytes(at(0x3c, 4)?.try_into().unwrap_or([0; 4])) as usize;
    if lfanew + 6 > bytes.len() || &bytes[lfanew..lfanew + 4] != b"PE\0\0" {
        return Err(StkError::simple(Domain::Filter, 42, "PE 签名不合法"));
    }
    let machine = u16::from_le_bytes([bytes[lfanew + 4], bytes[lfanew + 5]]);
    Ok(match machine {
        0x8664 => Arch::X64,
        0x14c => Arch::X86,
        0xaa64 => Arch::Unknown, // ARM64：本项目不支持，明确拒绝而不是当成 x64
        _ => Arch::Unknown,
    })
}

/// 构造一个仅供测试用的最小 PE 头。
#[cfg(test)]
fn fake_pe(machine: u16, lfanew: usize) -> Vec<u8> {
    let mut v = vec![0u8; lfanew + 6];
    v[0] = b'M';
    v[1] = b'Z';
    v[0x3c..0x40].copy_from_slice(&(lfanew as u32).to_le_bytes());
    v[lfanew..lfanew + 4].copy_from_slice(b"PE\0\0");
    v[lfanew + 4..lfanew + 6].copy_from_slice(&machine.to_le_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn csri_provider_is_recognized() {
        let r = probe(
            "XySubFilter.dll",
            &ex(&[
                "csri_open",
                "csri_close",
                "csri_render",
                "csri_get_renderer",
            ]),
        );
        assert_eq!(r.kind, Some(ProviderKind::Csri));
        assert_eq!(r.stage(ProbeStage::CsriCapability), Some(Outcome::Pass));
        // CSRI 命中后不再尝试 legacy（两条链不混）
        assert_eq!(
            r.stage(ProbeStage::LegacyCapability),
            Some(Outcome::NotApplicable)
        );
    }

    #[test]
    fn missing_one_csri_symbol_is_not_csri() {
        let r = probe(
            "XySubFilter.dll",
            &ex(&["csri_open", "csri_close", "csri_render"]),
        );
        assert_eq!(r.stage(ProbeStage::CsriCapability), Some(Outcome::Fail));
        assert_ne!(r.kind, Some(ProviderKind::Csri));
        assert!(r
            .stages
            .iter()
            .any(|(_, _, msg)| msg.contains("csri_get_renderer")));
    }

    #[test]
    fn filename_is_never_the_basis() {
        // 名字就叫 VSFilterMod.dll，但没有 CSRI 导出 → 不得判为 CSRI provider
        let r = probe(
            "VSFilterMod.dll",
            &ex(&["getVapourSynthAPI", "vsfm.TextSubMod"]),
        );
        assert_eq!(r.kind, Some(ProviderKind::LegacyVapourSynth));
        assert_eq!(r.stage(ProbeStage::CsriCapability), Some(Outcome::Fail));
        // 同名的 CSRI build 才会走 CSRI
        let c = probe("VSFilterMod.dll", &ex(&CSRI_REQUIRED_EXPORTS));
        assert_eq!(c.kind, Some(ProviderKind::Csri));
        // 名字像 provider 但既无 CSRI 也无 VS/AVS 符号 → 不给 kind，绝不猜
        let n = probe(
            "VSFilterMod.dll",
            &ex(&["init_vsfm", "getVapourSynthPlugin"]),
        );
        assert_eq!(n.kind, None);
    }

    #[test]
    fn avs_exports_map_to_avisynth_chain() {
        let r = probe(
            "textsub.avs.dll",
            &ex(&["avs_create_script_environment", "TextSub"]),
        );
        assert_eq!(r.kind, Some(ProviderKind::LegacyAviSynth));
    }

    #[test]
    fn empty_exports_fails_everything() {
        let r = probe("mystery.dll", &[]);
        assert_eq!(r.kind, None);
        assert_eq!(r.stage(ProbeStage::ExportedApi), Some(Outcome::Fail));
    }

    #[test]
    fn pe_arch_reads_machine() {
        assert_eq!(pe_arch(&fake_pe(0x8664, 0x100)).unwrap(), Arch::X64);
        assert_eq!(pe_arch(&fake_pe(0x014c, 0x80)).unwrap(), Arch::X86);
        assert_eq!(pe_arch(&fake_pe(0xaa64, 0x80)).unwrap(), Arch::Unknown);
        assert!(pe_arch(b"not a pe").is_err());
        assert!(pe_arch(&[]).is_err());
        let mut truncated = fake_pe(0x8664, 0x100);
        truncated.truncate(0x104);
        assert!(pe_arch(&truncated).is_err());
        let mut bad_sig = fake_pe(0x8664, 0x100);
        bad_sig[0x100] = b'X';
        assert!(pe_arch(&bad_sig).is_err());
    }
}
