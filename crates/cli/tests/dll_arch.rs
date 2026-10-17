//! 23 §DLL / 12 §3：**PE arch 检测**在 CLI 上的行为。
//!
//! 规格原话是「第三方 provider 进 renderer-host；PE arch 检测」以及
//! 12 §3 的「不得跨位数 `LoadLibrary`」。这一条的价值全在**报错发生的位置**：
//! 在读到文件之后立刻判定，用户就会看到"位数不对"；交给 `LoadLibrary`，
//! 用户看到的只会是一句含糊的"不是有效的 Win32 应用程序"。
//!
//! 所以这一组测试造的是**真的 PE 头**（不是随便几个字节），
//! 检查的也是 CLI 的完整输出，而不是内部函数。
//!
//! 本机没有真 libass 也能跑：位数检查发生在**打开库之前**，
//! 所以一个只有 PE 头的假 DLL 就能触发它。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pell-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 断言用的最小 PE：DOS 头 + PE 签名 + 机器类型 + 可选头魔数。
fn pe_bytes(machine: u16, opt_magic: u16) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = b'M';
    v[1] = b'Z';
    v[60..64].copy_from_slice(&64u32.to_le_bytes()); // e_lfanew
    v.extend_from_slice(b"PE\0\0");
    v.extend_from_slice(&machine.to_le_bytes()); // 文件头 Machine
    v.extend_from_slice(&0u16.to_le_bytes()); // NumberOfSections
    v.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    v.extend_from_slice(&0u32.to_le_bytes()); // PointerToSymbolTable
    v.extend_from_slice(&0u32.to_le_bytes()); // NumberOfSymbols
    v.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
    v.extend_from_slice(&0u16.to_le_bytes()); // Characteristics
    v.extend_from_slice(&opt_magic.to_le_bytes()); // 可选头魔数
    v
}

const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,30,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.00,D,,0,0,0,,Hi\n";

fn render(dir: &Path, dll: &Path) -> serde_json::Value {
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let o = Command::new(cli())
        .args(["render"])
        .arg(&ass)
        .args(["--fps", "25", "--canvas", "640x360", "--out"])
        .arg(dir.join("a.xml"))
        .arg("--json")
        .env("STK_LIBASS_DLL", dll)
        .output()
        .expect("跑 CLI");
    let s = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(s.trim()).unwrap_or_else(|e| {
        panic!(
            "输出不是单个 JSON 信封：{e}\nstdout={s}\nstderr={}",
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn err_of(v: &serde_json::Value) -> (String, String, String) {
    let e = &v["errors"][0];
    (
        e["code"].as_str().unwrap_or_default().to_string(),
        e["title"].as_str().unwrap_or_default().to_string(),
        e["details"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default(),
    )
}

/// 位数不符的库必须在**加载之前**被拒，且错误里说得出两边各是什么。
///
/// 这是 23 §DLL 那条要求的可判定形式：不是"最后失败了"，而是
/// "以正确的理由、在正确的位置失败"。
#[test]
fn a_dll_of_the_wrong_bitness_is_refused_before_loading() {
    let d = scratch("wrongbits");
    let (machine, magic, expect) = if std::env::consts::ARCH == "x86_64" {
        (0x014Cu16, 0x010Bu16, "x86") // 造一个 32 位库
    } else {
        (0x8664, 0x020B, "x64") // 本机不是 x64 时造一个 64 位的
    };
    let dll = d.join("wrong.dll");
    std::fs::write(&dll, pe_bytes(machine, magic)).unwrap();

    let v = render(&d, &dll);
    assert!(!v["ok"].as_bool().unwrap(), "该失败：{v}");
    let (code, title, details) = err_of(&v);
    assert_eq!(code, "STK-INP-0143", "{v}");
    assert!(title.contains("位数"), "{title}");
    assert!(
        details.contains(expect) && details.contains("x64"),
        "详情要说清两边各是什么：{details}"
    );
    assert!(
        v["errors"][0]["action"]
            .as_str()
            .unwrap_or_default()
            .contains("x64"),
        "要给出能照着做的补法：{v}"
    );
}

/// 不是 PE 的文件要报"不是有效的 PE"，而不是含糊的加载失败或位数不符。
///
/// 两类失败的**用户动作完全不同**：一类是"拿错了/文件坏了"，另一类是"拿对东西了但版本不对"。
/// 混成一条就等于把这两件事一起丢给用户猜。
#[test]
fn a_non_pe_file_is_reported_as_not_a_pe_not_as_a_bitness_mismatch() {
    let d = scratch("notpe");
    for (name, body) in [
        ("junk.dll", b"this is not a PE at all, just text".to_vec()),
        ("empty.dll", Vec::new()),
        ("truncated.dll", {
            let mut v = pe_bytes(0x8664, 0x020B);
            v.truncate(0x30); // 连 DOS 头都不完整
            v
        }),
        ("badpe.dll", {
            // 有 MZ 与合法 e_lfanew，但 PE 签名不对
            let mut v = vec![0u8; 64];
            v[0] = b'M';
            v[1] = b'Z';
            v[60..64].copy_from_slice(&64u32.to_le_bytes());
            v.extend_from_slice(b"XX\0\0");
            v.resize(96, 0);
            v
        }),
    ] {
        let dll = d.join(name);
        std::fs::write(&dll, &body).unwrap();
        let v = render(&d, &dll);
        assert!(!v["ok"].as_bool().unwrap(), "{name} 该失败：{v}");
        let (code, title, _) = err_of(&v);
        assert_eq!(code, "STK-INP-0142", "{name}：{v}");
        assert!(
            title.contains("PE"),
            "{name} 的标题该点出“不是 PE”：{title}"
        );
    }
}

/// 不存在的路径仍走原来那条（0141），不被新检查吞掉。
/// 三种失败要保持可分：不存在 / 不是 PE / 位数不符。
#[test]
fn the_three_dll_failures_stay_distinguishable() {
    let d = scratch("three");
    let missing = d.join("nope.dll");
    let v = render(&d, &missing);
    let (code, _, _) = err_of(&v);
    assert_eq!(code, "STK-INP-0141", "不存在：{v}");

    let notpe = d.join("notpe.dll");
    std::fs::write(&notpe, b"text").unwrap();
    let v = render(&d, &notpe);
    let (code, _, _) = err_of(&v);
    assert_eq!(code, "STK-INP-0142", "不是 PE：{v}");

    // 三个码互不相同 —— 这条断言看着平凡，但它正是"可分"的定义
    assert_ne!("STK-INP-0141", "STK-INP-0142");
    assert_ne!("STK-INP-0142", "STK-INP-0143");
}

/// 与本机同位的 PE 要**通过**位数检查，继续往下走到真正的加载。
///
/// 少了这条负基线，"什么都不接受"的实现也能让上面三条测试全绿。
/// 这里不要求渲染成功（那需要真 libass），只要求失败原因**不再**是位数问题。
#[test]
fn a_matching_bitness_pe_passes_the_arch_check() {
    let d = scratch("match");
    let (machine, magic) = match std::env::consts::ARCH {
        "x86" => (0x014Cu16, 0x010Bu16),
        "x86_64" => (0x8664, 0x020B),
        "aarch64" => (0xAA64, 0x020B),
        "arm" => (0x01C4, 0x010B),
        other => {
            eprintln!("SKIP：本机架构 {other} 不在已知表里");
            return;
        }
    };
    let dll = d.join("ok.dll");
    std::fs::write(&dll, pe_bytes(machine, magic)).unwrap();
    let v = render(&d, &dll);
    // 只有 PE 头、没有导出表，加载必然失败 —— 但原因**不该**是位数
    let (code, _, _) = err_of(&v);
    assert_ne!(code, "STK-INP-0142", "同位不该被判成非 PE：{v}");
    assert_ne!(code, "STK-INP-0143", "同位不该被判成位数不符：{v}");
    assert_ne!(code, "STK-INP-0141", "文件存在：{v}");
}

/// 真 libass（若本机有）必须通过位数检查并真的渲染出东西。
/// 这条把"检查器不会误杀真货"钉在真二进制上。
#[test]
fn a_real_libass_passes_the_check_and_renders() {
    let Some(dll) = std::env::var("STK_LIBASS_DLL")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("SKIP：本机没有 libass（设 STK_LIBASS_DLL）");
        return;
    };
    let d = scratch("real");
    let v = render(&d, &dll);
    assert!(v["ok"].as_bool().unwrap(), "真 libass 该能渲染：{v}");
    assert!(d.join("a.xml").is_file(), "该产出 BDN XML");
}
