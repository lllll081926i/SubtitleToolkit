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

/// CSRI 必需导出名集合。
///
/// ## 第一版把这四个名字写错了
///
/// 原来的集合是 `[csri_open, csri_close, csri_render, csri_get_renderer]`，
/// 其中 **`csri_open` 与 `csri_get_renderer` 根本不是 CSRI API 的一部分**。
/// 后果是：**每一个真的 CSRI provider 都会被判成"不是 CSRI provider"** ——
/// 而这四个名字还会出现在错误信息里，让人以为是自己拿错了 DLL。
///
/// 实测抳出来的：本机 `third_party\VSFilterMod.dll` 导出 8 个 `csri_*` 符号，
/// 但 **没有** `csri_open`。按旧集合它会被误判为非 provider。
///
/// ## 取集合的原则：只要“能建实例 + 能画一帧 + 能释放”
///
/// | 符号 | 作用 |
/// | --- | --- |
/// | `csri_renderer_default` | 拿到 renderer 句柄 |
/// | `csri_open_file` \| `csri_open_mem` | 从脚本建实例（**二选一**） |
/// | `csri_render` | 画一帧 |
/// | `csri_close` | 释放实例 |
///
/// ## 为什么 **不** 要求 `csri_request_fmt`
///
/// 这是本轮**差点搞错的一步**，值得写下来：
///
/// 上游 `csri.h` 里确实有 `csri_request_fmt(inst, &fmt)`，而且旧式调用序列
/// （设格式 → 画帧）要用它。所以我一度把它加进了必需集。
///
/// 但**实测否掉了这个改动**：本机真实 `VSFilterMod.dll` 导出 20 个名字，
/// 其中有 `csri_query_ext` 而**没有** `csri_request_fmt`。
/// 原因正好对应 12 §1B 那句“只有确实暴露标准 CSRI / `csri_stream_ext`
/// 的 provider 才走这条链” —— 新式的 `csri_stream_ext` 路径把
/// “设格式”挪进了扩展接口，不再需要那个旧入口。
///
/// 所以把它列为必需项会把**恰恰是规格点名的那类 provider** 排除掉。
/// 它归入 [`CSRI_OPTIONAL_EXPORTS`]。
///
/// **教训**：能力集合不能从“头文件里有什么”推，要从
/// “**真的 provider 导了什么**”反推。头文件描述接口全集，
/// 而实现会选其中一条路径。
///
/// 见 `docs/history/2026-10-06/ISSUES.md` C9。
pub const CSRI_REQUIRED_EXPORTS: [&str; 3] = ["csri_renderer_default", "csri_render", "csri_close"];

/// `csri_open_file` 与 `csri_open_mem` **至少有一个**。
///
/// 分成两半而不是写进上面那个集合：它们不是"两个都要"，
/// 而是"两条等价入口的二选一"。写成一个固定列表就无法表达这个语义。
///
/// 实测：VSFilterMod 两个都导；而一个只支持内存脚本的 provider
/// （宿主把 ASS 内容直接喂进去）只导 `csri_open_mem` —— 它也是可用的 provider，
/// 判它不可用是假的阴性。
pub const CSRI_OPEN_VARIANTS: [&str; 2] = ["csri_open_file", "csri_open_mem"];

/// 可选但有意义的 CSRI 导出：**不参与“是不是 provider”的判定**，
/// 但用户应当知道它们在与不在。
///
/// `csri_query_ext` 是查扩展的入口，`csri_stream_ext`（12 §1B 点名的那条）
/// 就是通过它获得的。
///
/// `csri_request_fmt` 在这里而不在必需集里：旧式路径用它设格式，
/// 而新式的 `csri_stream_ext` 路径把这件事挪进扩展接口了 ——
/// 实测 VSFilterMod 就只有后者。详见 [`CSRI_REQUIRED_EXPORTS`] 的注释。
pub const CSRI_OPTIONAL_EXPORTS: [&str; 6] = [
    "csri_query_ext",
    "csri_request_fmt",
    "csri_renderer_info",
    "csri_renderer_next",
    "csri_renderer_byname",
    "csri_renderer_byext",
];

/// 一个导出集里**有**哪些可选项（按 [`CSRI_OPTIONAL_EXPORTS`] 的顺序）。
pub fn csri_optional_extensions(exports: &[String]) -> Vec<&'static str> {
    CSRI_OPTIONAL_EXPORTS
        .iter()
        .copied()
        .filter(|n| exports.iter().any(|e| e == n))
        .collect()
}

/// 只看导出名，判定"这是不是一个 CSRI provider"。
///
/// 给 `capability::probe` 与错误文案共用，避免两处各自解释一遍"什么叫 CSRI"。
pub fn looks_like_csri(exports: &[String]) -> bool {
    let has = |n: &str| exports.iter().any(|e| e == n);
    CSRI_REQUIRED_EXPORTS.iter().all(|n| has(n)) && CSRI_OPEN_VARIANTS.iter().any(|n| has(n))
}

/// 列出**缺哪个必需项**（用于错误文案）。空 = 齐全。
pub fn missing_csri(exports: &[String]) -> Vec<&'static str> {
    let has = |n: &str| exports.iter().any(|e| e == n);
    let mut out: Vec<&'static str> = CSRI_REQUIRED_EXPORTS
        .iter()
        .copied()
        .filter(|n| !has(n))
        .collect();
    if !CSRI_OPEN_VARIANTS.iter().any(|n| has(n)) {
        out.push("csri_open_file（或 csri_open_mem）");
    }
    out
}

/// 探测主流程。`exports` 由 PE 解析层提供（不在本 crate 里读 DLL，保持可测试）。
pub fn probe(filename_hint: &str, exports: &[String]) -> ProbeResult {
    let mut stages: Vec<(ProbeStage, Outcome, String)> = Vec::new();
    let missing = missing_csri(exports);
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
            // 把**缺哪个**写出来：只说“缺 CSRI 符号”会让用户无从判断
            // 是自己拿错了 DLL，还是这个 provider 用了另一条接口。
            format!("缺少 CSRI 符号: {missing:?}")
        },
    ));
    let legacy = if has_csri {
        Outcome::NotApplicable
    } else {
        let vs = exports.iter().any(|e| {
            e.starts_with("getVapourSynthAPI")
                || e.starts_with("vsfm")
                || e == "VapourSynthPluginInit"
        });
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
    } else if exports.iter().any(|e| {
        e.starts_with("getVapourSynthAPI") || e.starts_with("vsfm") || e == "VapourSynthPluginInit"
    }) {
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
                "csri_renderer_default",
                "csri_render",
                "csri_close",
                "csri_open_file",
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

    /// 回归用例：**真实的 VSFilterMod 导出集必须被判为 CSRI。**
    ///
    /// 这组名字是从本机 `third_party\VSFilterMod.dll` 的**真实导出表**抄来的
    /// （20 个名字里的 csri_* 部分）。两点值得看：
    ///
    /// 1. 它没有 `csri_open` —— 第一版的必需集合要求那个名字，
    ///    于是**每一个真的 CSRI provider 都会被误判**。
    /// 2. 它**也没有** `csri_request_fmt` —— 本轮我一度把它加进必需集，
    ///    这条用例当场报了。它走的是 `csri_query_ext` / `csri_stream_ext`
    ///    那条新式路径（正是 12 §1B 点名的那条）。
    #[test]
    fn the_real_vsfiltermod_exports_are_recognized_as_csri() {
        let real = ex(&[
            "VapourSynthPluginInit",
            "csri_close",
            "csri_open_file",
            "csri_open_mem",
            "csri_query_ext",
            "csri_render",
            "csri_renderer_byname",
            "csri_renderer_default",
            "csri_renderer_info",
        ]);
        let r = probe("VSFilterMod.dll", &real);
        assert_eq!(
            r.kind,
            Some(ProviderKind::Csri),
            "真实 CSRI 导出集被误判了：{:?}",
            r.stages
        );
        // 而它含哪些可选项要能报出来 —— 包括那个说明它走 stream_ext 的
        // `csri_query_ext`，以及**不**含 `csri_request_fmt` 这个事实。
        let opt = csri_optional_extensions(&real);
        assert!(
            opt.contains(&"csri_query_ext"),
            "应当报出 csri_query_ext: {opt:?}"
        );
        assert!(
            !opt.contains(&"csri_request_fmt"),
            "VSFilterMod 不导 csri_request_fmt（走 stream_ext 路径）: {opt:?}"
        );
    }

    /// **内存入口也能算 CSRI provider**。
    ///
    /// 回归用例：必需集合里若把 `csri_open_file` 写成硬要求，
    /// 一个只支持内存脚本的 provider 会被误判成不可用 —— 那是假阴性。
    #[test]
    fn a_memory_only_csri_provider_is_still_csri() {
        let r = probe(
            "mem-only.dll",
            &ex(&[
                "csri_renderer_default",
                "csri_render",
                "csri_close",
                "csri_open_mem",
            ]),
        );
        assert_eq!(r.kind, Some(ProviderKind::Csri), "内存入口也是合法入口");
    }

    /// `csri_request_fmt` **不得**成为必需项。
    ///
    /// 本轮差点搞错：它在上游 `csri.h` 里确实存在，所以看着像必需。
    /// 但它是**旧式**路径的入口，而采用 `csri_stream_ext` 的 provider
    /// （包括 VSFilterMod）不导出它 —— 把它做成硬要求，
    /// 会把**规格点名的那类 provider** 排除掉。
    #[test]
    fn csri_request_fmt_is_not_required() {
        assert!(
            !CSRI_REQUIRED_EXPORTS.contains(&"csri_request_fmt"),
            "旧式入口不是必需项（新式 stream_ext 路径不用它）"
        );
        assert!(CSRI_OPTIONAL_EXPORTS.contains(&"csri_request_fmt"));
        // 一个不导它的 provider 仍然算 CSRI
        let r = probe(
            "stream-ext-only.dll",
            &ex(&[
                "csri_renderer_default",
                "csri_open_mem",
                "csri_render",
                "csri_close",
                "csri_query_ext",
            ]),
        );
        assert_eq!(r.kind, Some(ProviderKind::Csri));
    }

    /// 旧集合里的那两个幽灵名字**不得**成为必需项。
    ///
    /// 直接钉住“含 `csri_open` 而缺 `csri_renderer_default` 的东西不是 CSRI provider”。
    #[test]
    fn the_phantom_symbols_are_not_required() {
        assert!(
            !CSRI_REQUIRED_EXPORTS.contains(&"csri_open"),
            "csri_open 不是 CSRI API 的一部分"
        );
        assert!(
            !CSRI_REQUIRED_EXPORTS.contains(&"csri_get_renderer"),
            "csri_get_renderer 不是 CSRI API 的一部分"
        );
        // 只导出幽灵名字的那一组不能算 CSRI
        let r = probe(
            "fake.dll",
            &ex(&[
                "csri_open",
                "csri_close",
                "csri_render",
                "csri_get_renderer",
            ]),
        );
        assert_ne!(r.kind, Some(ProviderKind::Csri));
    }

    #[test]
    fn missing_one_csri_symbol_is_not_csri() {
        let r = probe(
            "XySubFilter.dll",
            &ex(&["csri_renderer_default", "csri_render", "csri_open_file"]),
        );
        assert_eq!(r.stage(ProbeStage::CsriCapability), Some(Outcome::Fail));
        assert_ne!(r.kind, Some(ProviderKind::Csri));
        assert!(r
            .stages
            .iter()
            .any(|(_, _, msg)| msg.contains("csri_close")));
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
        // 同名的 CSRI build 才会走 CSRI（必需项 + 任一个 open 变体）
        let mut csri_exports: Vec<&str> = CSRI_REQUIRED_EXPORTS.to_vec();
        csri_exports.push(CSRI_OPEN_VARIANTS[0]);
        let c = probe("VSFilterMod.dll", &ex(&csri_exports));
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
