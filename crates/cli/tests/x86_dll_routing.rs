//! AT-RENDER-003：**x86 DLL 路由**。
//!
//! ## 规格原文
//!
//! > AT-RENDER-003：仅在 x86 host 产物加入支持矩阵后，验证 x86 DLL 路由；
//! > 当前缺少该产物，不验收自动路由。
//!
//! ## 这一条为什么**至今仍不能验收** —— 说清楚，免得被当成已做
//!
//! 规格要的是"**验证 x86 DLL 路由**"，即：给一个真 x86 libass DLL，
//! 系统把它送进 x86 host，并对 x64 主进程提供可用的渲染。
//!
//! 那需要一只**真的 x86 libass DLL**。本机扫过三个 `libass*.dll`
//! （`third_party\libass-9.dll`、`target\native-msys2-*\ucrt64\bin\libass-9.dll`、
//! `D:\OncePower\libass.dll`），**全是 x64**（机器类型 0x8664）。
//! 而"造一个能渲染的 x86 DLL"不是测试，是伪造 —— 一个空壳 DLL
//! 能过位数校验却渲染不出任何东西，那种"通过"没有意义。
//!
//! 所以本文件**只验路由判定本身**（我们确实控制得了的那部分），
//! 并把"端到端 x86 渲染"如实记为未验收。
//!
//! ## 本文件验什么
//!
//! 判定链上有四处会出错，每一处都能用**手造的 x86 PE** 精确触发：
//!
//! 1. **位数识别**：一个真 x86 PE 必须被读成 `x86`，而不是当成坏文件或 x64；
//! 2. **拒绝跨位数**：x64 主进程拿到 x86 DLL 时必须报"位数不符"，
//!    而**不是**含糊的"不是有效的 Win32 应用程序"（那是 `LoadLibrary` 的说法）；
//! 3. **不许先加载再看**：12 §3 禁止跨位数 `LoadLibrary`。判据是
//!    **错误的归属** —— 见第 3 条用例的说明；
//! 4. **x86 PE 的能力探测不加载**：`provider probe` 对 x86 DLL 要能报出
//!    导出与位数，且明确说自己"没有加载"。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("x86route-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 造一个**带真导出表**的 PE，机器类型可控。
///
/// 布局照 `core-model::pe` 自己测试里那套已被验证的写法（节起点 0x1000 RVA、
/// 原始数据在 0x400），只多一个 `machine` 参数 —— 因为本文件要的就是
/// “同样的形状、不同的位数”。
///
/// 为什么不能只用“只有 PE 头”的假 PE：`provider probe` 要读导出表，
/// 只有头的文件读出来是“没有导出”，那就验不出
/// “x86 libass 形状的 DLL 会被怎么归类”。
fn pe_with_exports(machine: u16, names: &[&str]) -> Vec<u8> {
    const OPT_AT: usize = 0x98;
    const OPT_SIZE: usize = 0xf0;
    const SEC_AT: usize = OPT_AT + 24 + OPT_SIZE;
    const SEC_RAW: usize = 0x400;
    const EXP_RVA: u32 = 0x1000;
    const PE_LEAN: [u8; 4] = [0x50, 0x45, 0x00, 0x00];

    let mut v = vec![0u8; SEC_RAW + 0x1000];
    v[0] = b'M';
    v[1] = b'Z';
    v[0x3c..0x40].copy_from_slice(&(OPT_AT as u32).to_le_bytes());
    v[OPT_AT..OPT_AT + 4].copy_from_slice(&PE_LEAN);
    v[OPT_AT + 4..OPT_AT + 6].copy_from_slice(&machine.to_le_bytes());
    v[OPT_AT + 6..OPT_AT + 8].copy_from_slice(&1u16.to_le_bytes());
    v[OPT_AT + 20..OPT_AT + 22].copy_from_slice(&(OPT_SIZE as u16).to_le_bytes());
    // x64 用 PE32+ 魔数，x86 用 PE32 —— 两者可选头大小不同，
    // 而这一处写错会让 `arch_of_bytes` 或导出表读到别的字段上去。
    let is64 = machine == 0x8664;
    let magic: u16 = if is64 { 0x020b } else { 0x010b };
    v[OPT_AT + 24..OPT_AT + 26].copy_from_slice(&magic.to_le_bytes());
    // 数据目录 0 = Export（PE32+ 在可选头 +112，PE32 在 +96）
    let dd = OPT_AT + 24 + if is64 { 112 } else { 96 };
    v[dd..dd + 4].copy_from_slice(&EXP_RVA.to_le_bytes());
    v[dd + 4..dd + 8].copy_from_slice(&0x200u32.to_le_bytes());
    v[SEC_AT + 8..SEC_AT + 12].copy_from_slice(&0x1000u32.to_le_bytes());
    v[SEC_AT + 12..SEC_AT + 16].copy_from_slice(&EXP_RVA.to_le_bytes());
    v[SEC_AT + 16..SEC_AT + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    v[SEC_AT + 20..SEC_AT + 24].copy_from_slice(&(SEC_RAW as u32).to_le_bytes());

    let exp = SEC_RAW;
    let n = names.len() as u32;
    let names_tbl = exp + 40;
    let ord_tbl = names_tbl + (n as usize) * 4;
    let strs = ord_tbl + (n as usize) * 2;
    v[exp + 24..exp + 28].copy_from_slice(&n.to_le_bytes());
    v[exp + 32..exp + 36].copy_from_slice(&(EXP_RVA + 40).to_le_bytes());
    v[exp + 36..exp + 40].copy_from_slice(&(EXP_RVA + 40 + n * 4).to_le_bytes());
    let mut cur = strs;
    for (i, name) in names.iter().enumerate() {
        let rva = EXP_RVA + (cur - SEC_RAW) as u32;
        v[names_tbl + i * 4..names_tbl + i * 4 + 4].copy_from_slice(&rva.to_le_bytes());
        v[ord_tbl + i * 2..ord_tbl + i * 2 + 2].copy_from_slice(&(i as u16).to_le_bytes());
        v[cur..cur + name.len()].copy_from_slice(name.as_bytes());
        cur += name.len() + 1;
    }
    v
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(cli()).args(args).output().expect("跑 CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code(),
    )
}

const LIBASS_EXPORTS: &[&str] = &[
    "ass_library_init",
    "ass_library_done",
    "ass_renderer_init",
    "ass_renderer_done",
    "ass_new_track",
    "ass_read_memory",
    "ass_render_frame",
    "ass_set_frame_size",
    "ass_set_fonts",
];

fn write_x86_libass_shape(dir: &Path) -> PathBuf {
    let p = dir.join("fake-x86-libass.dll");
    std::fs::write(&p, pe_with_exports(0x014C, LIBASS_EXPORTS)).unwrap();
    p
}

fn write_x64_libass_shape(dir: &Path) -> PathBuf {
    let p = dir.join("fake-x64-libass.dll");
    std::fs::write(&p, pe_with_exports(0x8664, LIBASS_EXPORTS)).unwrap();
    p
}

fn write_blank_ass(dir: &Path) -> PathBuf {
    let p = dir.join("a.ass");
    std::fs::write(
        &p,
        "[Script Info]\nScriptType: v4.00+\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    )
    .unwrap();
    p
}

/// 1. 手造的 x86 PE 必须被读成 x86，且导出名读得出来。
///
/// 这一条是**其余三条的地基**：如果连位数都读错，后面的"拒绝/路由"都无从谈起。
#[test]
fn the_x86_fixture_is_really_an_x86_pe_with_readable_exports() {
    let dir = scratch("fixture");
    let p = write_x86_libass_shape(&dir);

    let arch = core_model::pe::arch_of_file(&p).expect("应当能读出来");
    assert_eq!(arch.label(), "x86", "手造的应当是 x86");

    let ex = core_model::pe::exports_of_file(&p).expect("导出表能读");
    assert!(
        ex.names.iter().any(|n| n == "ass_render_frame"),
        "导出名要读得出来，实际：{:?}",
        ex.names
    );

    // 对照：同样的写法换机器类型就该读成 x64 —— 证明夹具真的区分位数
    let q = write_x64_libass_shape(&dir);
    assert_eq!(
        core_model::pe::arch_of_file(&q).unwrap().label(),
        "x64",
        "同样的写法换机器类型就该读成 x64"
    );
}

/// 2. **x64 主进程拿到 x86 DLL 必须明确报“位数不符”**，而不是
///    `LoadLibrary` 那句含糊的“不是有效的 Win32 应用程序”。
///
/// 考的是**报错位置**：在读到文件之后立刻判，用户看到“位数不对”；
/// 交给 `LoadLibrary`，用户只看到一句猜不出原因的错。
#[test]
fn an_x86_dll_requires_the_matching_x86_host() {
    let dir = scratch("route");
    let dll = write_x86_libass_shape(&dir);
    let ass = write_blank_ass(&dir);

    let (stdout, stderr, code) = run(&[
        "render",
        ass.to_str().unwrap(),
        "--libass-dll",
        dll.to_str().unwrap(),
        "--out",
        dir.join("o").to_str().unwrap(),
    ]);

    assert_ne!(code, Some(0), "本机未打包 x86 host 时必须失败");
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("x86") && all.contains("宿主"),
        "报错必须点出缺少匹配 x86 宿主，实际：{all}"
    );
    assert!(
        !all.contains("不是有效的 Win32"),
        "不许把“位数不符”退化成加载器的含糊错误：{all}"
    );
}

/// 3. **12 §3：不许先 `LoadLibrary` 再看。**
///
/// 判据用的是**错误的归属**：把导出目录的 RVA 改成一个不存在的位置，
/// 于是"读导出表"这条路会失败。断言仍然必须看到**位数**相关的说法 ——
/// 因为位数检查发生在**读导出表之前**，也就必然发生在任何加载之前。
///
/// 反过来说：如果实现是"先加载，加载失败了再猜"，那这里的报错文案
/// 就会是加载器/解析器的话，这条会失败。
#[test]
fn the_arch_check_happens_before_any_load_library() {
    let dir = scratch("no-load");
    let mut bytes = pe_with_exports(0x014C, LIBASS_EXPORTS);
    let e_lfanew =
        u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    let opt = e_lfanew + 4 + 20;
    let magic = u16::from_le_bytes([bytes[opt], bytes[opt + 1]]);
    let dd = opt + 24 + if magic == 0x020b { 112 } else { 96 };
    // 导出目录 RVA 指到不存在的地方
    bytes[dd..dd + 4].copy_from_slice(&0xDEAD_B000u32.to_le_bytes());
    let dll = dir.join("broken-exports-x86.dll");
    std::fs::write(&dll, &bytes).unwrap();
    let ass = write_blank_ass(&dir);

    let (stdout, stderr, code) = run(&[
        "render",
        ass.to_str().unwrap(),
        "--libass-dll",
        dll.to_str().unwrap(),
        "--out",
        dir.join("o").to_str().unwrap(),
    ]);

    let all = format!("{stdout}{stderr}");
    assert_ne!(code, Some(0));
    assert!(
        all.contains("x86") && all.contains("宿主"),
        "必须在加载前报告 x86 host 路由，而不是导出表/加载器的话：{all}"
    );
}

/// 4. `provider probe` 对一个 x86 libass 形状的 DLL：
///    - 报得出 x86，
///    - 报得出导出名，
///    - 明确说自己**没有加载**任何东西。
///
/// 第 3 点是 12 §3 的直接要求，而且在**探测**这个场景下最要紧 ——
/// 探测的整个意义就是"先看看，别加载"。
#[test]
fn provider_probe_reads_an_x86_dll_without_loading_it() {
    let dir = scratch("probe");
    let dll = write_x86_libass_shape(&dir);

    let (stdout, stderr, code) = run(&["provider", "probe", dll.to_str().unwrap()]);
    let all = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "探测本身不该失败：{all}");

    assert!(all.contains("x86"), "要报出 x86：{all}");
    assert!(
        all.contains("只读了 PE 头与导出表") && all.contains("没有加载"),
        "必须明说自己没有加载：{all}"
    );
    assert!(
        all.contains("\"arch_ok_for_x64_host\": false"),
        "x86 对 x64 host 应当标不可用：{all}"
    );
}

/// 5. **本机没有任何真 x86 libass DLL** —— 这条把"AT-RENDER-003 为何未验收"
///    变成可核验的事实，而不是一句托词。
///
/// 如果哪天真的出现了 x86 libass，这条会失败，提醒去把它接进验收。
#[test]
fn no_real_x86_libass_exists_on_this_machine_so_at_render_003_stays_unverified() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let candidates = [
        root.join("third_party/libass-9.dll"),
        PathBuf::from("D:/OncePower/libass.dll"),
    ];
    let mut found_x86 = Vec::new();
    for c in candidates.iter().filter(|p| p.is_file()) {
        if let Ok(a) = core_model::pe::arch_of_file(c) {
            if a.label() == "x86" {
                found_x86.push(c.clone());
            }
        }
    }
    assert!(
        found_x86.is_empty(),
        "本机出现了真的 x86 libass：{found_x86:?} —— \
         请把它接进 AT-RENDER-003 的端到端验收，并更新 at_coverage.py"
    );
}
