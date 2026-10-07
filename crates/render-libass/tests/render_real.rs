//! 真实 libass 渲染（12 §1A）。
//!
//! 这里证明的是**渲染语义**（像素、尺寸、时间轴、确定性）。它不等于 AT-RENDER-001 ——
//! 那三条考的是 renderer-host 的崩溃隔离（12 §2-§7），host 还没建（TRACEABILITY §18）。
//!
//! 需要 native 依赖。查找顺序：`STK_LIBASS_DLL` 环境变量 → 本机 vcpkg 安装目录。
//! 找不到时打印 SKIP；`STK_REQUIRE_LIBASS=1`（门阶设置）时直接失败——
//! "因为没跑所以通过"是我最不想见到的绿。

use core_model::canvas::Canvas;
use core_model::time::Time;
use image_core::Level;
use render_libass::Provider;
use std::path::PathBuf;

const ASS: &str = "[Script Info]\n\
ScriptType: v4.00+\n\
PlayResX: 1280\n\
PlayResY: 720\n\
ScaledBorderAndShadow: yes\n\
\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,72,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,20,20,40,1\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,Hello 世界\n\
";

fn dll_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        return None;
    }
    // 开发期默认：仓库同级 tools/vcpkg/installed（28 §1：native 依赖锁版本，不入库）
    let guess = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("tools")
        .join("vcpkg")
        .join("installed")
        .join("x64-windows")
        .join("bin")
        .join("ass-9.dll");
    guess.is_file().then_some(guess)
}

fn require_libass() -> bool {
    std::env::var("STK_REQUIRE_LIBASS").is_ok()
}

fn open(output: Canvas) -> Option<Provider> {
    let dll = dll_path()?;
    // 脚本 PlayRes 与输出画布分别设置（09 §2 的四层几何）
    Some(
        Provider::open(&dll, Canvas::HD720, output, "Arial")
            .unwrap_or_else(|e| panic!("打不开 {dll:?}：{e}｜细节 {e:?}")),
    )
}

/// 拿真实渲染器；本机没有 DLL 时按门阶要求决定是 SKIP 还是失败。
///
/// 每个用到 libass 的测试都必须走这个宏：第一版两个测试各写一份 SKIP 分支，
/// 其中一份忘了看 `STK_REQUIRE_LIBASS`，于是门阶下它"没跑也算通过"（ISSUES Z4）。
macro_rules! need_libass {
    ($output:expr) => {
        match open($output) {
            Some(p) => p,
            None if require_libass() => {
                panic!("STK_REQUIRE_LIBASS=1，但本机没有可用的 ass-9.dll（设 STK_LIBASS_DLL 指到它）")
            }
            None => {
                eprintln!("SKIP 真实渲染：本机没有 libass DLL");
                return;
            }
        }
    };
}

#[test]
fn libass_renders_text_into_rgba() {
    let provider = need_libass!(Canvas::HD720);
    eprintln!("libass library version = {}", provider.library_version());
    let script = provider
        .load_script(ASS.as_bytes())
        .expect("脚本必须能载入");

    let at = Time::from_millis(2000).unwrap();
    let r = render_libass::render(&script, at, Canvas::HD720).unwrap();
    eprintln!(
        "images={} detect={} 非零alpha像素={}",
        r.images,
        r.detect_change,
        r.frame.rgba.chunks_exact(4).filter(|p| p[3] != 0).count()
    );
    assert!(r.images > 0, "1~4 秒之间的事件必须画出位图");
    assert!(!r.frame.is_empty(), "渲染结果不能是空画布");
    let bbox = r.frame.bbox().expect("有内容就必须有 bbox");
    assert!(
        bbox.x + bbox.width <= Canvas::HD720.width && bbox.y + bbox.height <= Canvas::HD720.height,
        "内容必须落在输出画布内：{bbox:?}"
    );
    assert!(
        bbox.width > 40 && bbox.height > 10,
        "\"Hello 世界\" 在 72p/72px 下不可能只有 {}x{}",
        bbox.width,
        bbox.height
    );

    // 时间轴必须真的起作用：事件之外不画东西
    let outside =
        render_libass::render(&script, Time::from_millis(9000).unwrap(), Canvas::HD720).unwrap();
    assert_eq!(outside.images, 0, "9s 时事件已结束，不该有位图");
    assert!(outside.frame.is_empty(), "9s 的画布必须是全透明");

    // 同一帧重复渲染必须逐字节一致（golden/像素 diff 的前提）
    let again = render_libass::render(&script, at, Canvas::HD720).unwrap();
    assert_eq!(
        again.frame.rgba, r.frame.rgba,
        "同时间点重复渲染必须完全一致"
    );

    // 输出侧闭环：能写成真压缩 PNG，且 IHDR 就是输出画布（13 §7 / AT-BDN-001）
    let png = r.frame.to_png(Level::Balanced).unwrap();
    let hdr = image_core::header(&png).unwrap();
    assert_eq!((hdr.width, hdr.height), (1280, 720));
    assert!(png.len() * 4 < r.frame.rgba.len(), "PNG 必须是真压缩");
}

#[test]
fn output_canvas_change_is_not_a_rescale_of_the_same_pixels() {
    // 09 §2：frame_size 改变的是渲染目标，不是把 720p 位图放大成 1080p。
    // 这一条要 libass 自己参与才能证明，所以放在这里而不是 bitmap 单测里。
    let provider = need_libass!(Canvas::FHD1080);
    let script = provider.load_script(ASS.as_bytes()).unwrap();
    let at = Time::from_millis(2000).unwrap();
    let big = render_libass::render(&script, at, Canvas::FHD1080).unwrap();
    let bbox = big.frame.bbox().expect("1080p 也要有内容");
    assert!(
        bbox.height > 20,
        "1080p 画布上的字应当更大：{}",
        bbox.height
    );
    assert_eq!(
        (big.frame.canvas.width, big.frame.canvas.height),
        (1920, 1080)
    );
}

/// 现场造一个"这台机器上没有的家族"：复制系统里的一份字体，只改 name 表里的家族字符串
/// （保持每条记录的字节长度不变），再重算 name 表校验和与 `head.checkSumAdjustment`。
///
/// 为什么在测试里造而不是放一个字体文件进仓库：那是 Microsoft 的字体二进制，
/// 21 §1 的依赖口径只收宽松许可，把字体提交进来就是污染那条线（AG15 同族的判断）。
/// 返回（文件路径, 改出来的家族名）。
/// 现场造一份"这台机器上不存在的家族名"的字体：复制系统字体，把 name 表里的
/// 家族名改成 `wanted`（补下划线到原记录长度，保持字节数不变）。
///
/// 为什么要现场造：字体文件不入库（21 §1 的许可口径），但"注入的字节就是画像素的
/// 字节"这件事只有拿真字体才证得出来（ISSUES AK1）。
fn renamed_font(
    dir: &std::path::Path,
    stem: &str,
    src_name: &str,
    wanted: &str,
) -> Option<(PathBuf, String)> {
    let src = PathBuf::from(std::env::var("WINDIR").ok()?)
        .join("Fonts")
        .join(src_name);
    if !src.is_file() {
        return None;
    }
    let mut d = std::fs::read(&src).ok()?;
    // sfnt 里所有整数都是大端。表目录每项 16 字节：tag(4) checksum(4) offset(4) length(4)。
    // （这两处最容易写错：字段起点算偏一格，读到的"偏移"其实是"长度"，
    //   文件照样打开、内容全是别人的 —— 与 AH1 的 RegQueryInfoKey 同一族。）
    let num = u16::from_be_bytes([d[4], d[5]]) as usize;
    let mut name_tab: Option<(usize, usize, usize)> = None;
    let mut head_tab: Option<(usize, usize)> = None;
    for i in 0..num {
        let e = 12 + i * 16;
        let tag = &d[e..e + 4];
        let off = u32::from_be_bytes([d[e + 8], d[e + 9], d[e + 10], d[e + 11]]) as usize;
        let len = u32::from_be_bytes([d[e + 12], d[e + 13], d[e + 14], d[e + 15]]) as usize;
        if tag == b"name" {
            name_tab = Some((e, off, len));
        } else if tag == b"head" {
            head_tab = Some((off, len));
        }
    }
    let (name_entry, noff, nlen) = name_tab?;
    let (hoff, _hlen) = head_tab?;
    let count = u16::from_be_bytes([d[noff + 2], d[noff + 3]]) as usize;
    let storage = noff + u16::from_be_bytes([d[noff + 4], d[noff + 5]]) as usize;
    let mut made = String::new();
    for i in 0..count {
        let r = noff + 6 + i * 12;
        let pid = u16::from_be_bytes([d[r], d[r + 1]]);
        let nid = u16::from_be_bytes([d[r + 6], d[r + 7]]);
        let len = u16::from_be_bytes([d[r + 8], d[r + 9]]) as usize;
        let o = u16::from_be_bytes([d[r + 10], d[r + 11]]) as usize;
        // 只动 Windows 平台、长度够放一个唯一名字、且是偶数字节（UTF-16）的记录
        if pid != 3 || len < 8 || len % 2 != 0 {
            continue;
        }
        let chars = len / 2;
        let mut family = wanted.to_string();
        if family.chars().count() > chars {
            family = family.chars().take(chars).collect();
        }
        while family.chars().count() < chars {
            family.push('_');
        }
        let enc: Vec<u8> = family
            .encode_utf16()
            .flat_map(|c| c.to_be_bytes())
            .collect();
        if enc.len() != len {
            continue;
        }
        d[storage + o..storage + o + len].copy_from_slice(&enc);
        if nid == 1 {
            made = family;
        }
    }
    if made.is_empty() {
        return None;
    }
    fn sum32(bytes: &[u8]) -> u32 {
        let mut s: u64 = 0;
        for c in bytes.chunks(4) {
            let mut b = [0u8; 4];
            b[..c.len()].copy_from_slice(c);
            s = s.wrapping_add(u32::from_be_bytes(b) as u64);
        }
        (s & 0xFFFF_FFFF) as u32
    }
    let name_sum = sum32(&d[noff..noff + nlen]).to_be_bytes();
    d[name_entry + 4..name_entry + 8].copy_from_slice(&name_sum);
    // head.checkSumAdjustment：先置零求整文件和，再写 0xB1B0AFBA - 和
    d[hoff + 8..hoff + 12].copy_from_slice(&0u32.to_be_bytes());
    let total = sum32(&d);
    d[hoff + 8..hoff + 12].copy_from_slice(&0xB1B0_AFBAu32.wrapping_sub(total).to_be_bytes());
    let dest = dir.join(format!("{stem}.ttf"));
    std::fs::write(&dest, &d).ok()?;
    Some((dest, made))
}

/// 注入测试用的默认样本：marlett 改名成 `StkProbe…`。
fn probe_font(dir: &std::path::Path) -> Option<(PathBuf, String)> {
    renamed_font(dir, "probe", "marlett.ttf", "StkProbe")
}

fn ass_with(family: &str, text: &str) -> String {
    format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1280\nPlayResY: 720\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,{family},72,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,20,20,40,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:04.00,D,,0,0,0,,{text}\n"
    )
}

/// 17 §7 的正证：**交给 `ass_add_font` 的那批字节，就是画出来的那批字节。**
///
/// 为什么这条测试值得存在：注入之前，"渲染缓存 key 里钉住的字体哈希"只是我们
/// 自己按名字判断的结论，libass 在 Windows 上仍然按 DirectWrite 的选择去拿字体（AI4）。
/// 这里用一个这台机器上不可能存在的家族名（现场造出来的 `StkProbe…`）把两件事分开：
/// 不注入 → 字体解析落到回退族，像素是回退的；注入那批字节 → 同一个请求解析到它自己，
/// 像素跟着字节变。这就是"内存字体参与匹配、且按字体表里的内部名匹配"的实测，
/// 也是 `add_font` 返回 void（没有返回码可查）之后唯一的证据来源。
#[test]
fn injected_font_bytes_are_what_the_renderer_draws_with() {
    let dir = std::env::temp_dir().join(format!("stk-inject-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let Some((font, family)) = probe_font(&dir) else {
        if require_libass() {
            panic!("STK_REQUIRE_LIBASS=1，但造不出注入用的改名字体（拿不到 WINDIR 下的 Fonts/marlett.ttf）");
        }
        eprintln!("SKIP 注入测试：本机没有 marlett.ttf 可改名");
        return;
    };
    let bytes = std::fs::read(&font).unwrap();
    let ass = ass_with(&family, "4");

    // 基线：这个家族没装过，libass 只能回退
    let plain = need_libass!(Canvas::HD720);
    let script = plain.load_script(ass.as_bytes()).unwrap();
    let before =
        render_libass::render(&script, Time::from_millis(2000).unwrap(), Canvas::HD720).unwrap();
    assert!(plain.injected().is_empty(), "没注入就该是空台账");

    // 同一批字节交给 libass，请求同一个家族名
    let mut injected = need_libass!(Canvas::HD720);
    injected.add_font(&family, &bytes).unwrap();
    let script2 = injected.load_script(ass.as_bytes()).unwrap();
    let after =
        render_libass::render(&script2, Time::from_millis(2000).unwrap(), Canvas::HD720).unwrap();

    let led = injected.injected();
    assert_eq!(led.len(), 1, "台账要记下这一次注入：{led:?}");
    assert_eq!(led[0].name, family);
    assert_eq!(led[0].bytes, bytes.len(), "台账报的字节数要说真话");
    assert_eq!(led[0].sha256.len(), 64, "内容哈希是 SHA-256 的十六进制");
    assert!(
        led[0].sha256.chars().all(|c| c.is_ascii_hexdigit()),
        "{:?}",
        led[0]
    );

    assert_ne!(
        before.frame.rgba, after.frame.rgba,
        "注入的字节必须真的参与渲染：回退像素与注入像素相同就说明注入是装饰"
    );
    assert!(
        after.frame.bbox().is_some() && before.frame.bbox().is_some(),
        "两边都该画出东西来"
    );
    // 换一批字节，台账必须换哈希（否则"锁死字节"这件事是假的）
    let mut other = need_libass!(Canvas::HD720);
    let mut second = bytes.clone();
    second.reverse(); // 只要不是同一批字节就够；能不能解析是 libass 的事
    other.add_font(&family, &second).unwrap();
    assert_ne!(
        other.injected()[0].sha256,
        led[0].sha256,
        "台账里的哈希由字节决定"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn add_font_refuses_bytes_that_cannot_be_handed_over() {
    let mut provider = need_libass!(Canvas::HD720);
    // 空字节：不是"注入了一份 0 字体的字体"，那是骗报告
    assert_eq!(
        provider.add_font("Arial", &[]).unwrap_err().code_string(),
        "STK-RND-0081"
    );
    // 名字里有 NUL：CString 建不出来，不能悄悄截断
    assert!(provider.add_font("A\0B", b"x").is_err());
    // 没有能力时的报错由 can_inject_fonts 说清；这台机器的 DLL 是有能力的
    eprintln!("本机 DLL 能注入吗 = {}", provider.can_inject_fonts());
}

/// 同一个内部家族名、两批不同的字节：**注册顺序不是裁决依据**（实测）。
///
/// 这条测试原本要证的是 `inject_plan` 注释里那句"先注册的内存字体胜出"。结果它被
/// 像素否定了：verdana 与 marlett 都改名成同一个家族后，两种注册顺序画出**同一份**
/// 像素，都是 verdana 那份（ISSUES AK8）。所以：
/// ①顺序在这里不产生赢家，"脚本自带的字体优先"不能靠注册顺序实现，注释里的说法已删；
/// ②我们能依赖的只有两件事——同名多份时赢家必定是**交出去的那几批字节之一**
/// （不是悄悄回退到系统字体），而且这个赢家不受注册顺序影响（结果因此是确定的）。
/// 断言只钉这两条，不钉"具体哪一份赢"——那是 libass 的选择，不是我们的契约。
#[test]
fn registration_order_is_not_a_tiebreaker_for_the_same_family() {
    let dir = std::env::temp_dir().join(format!("stk-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let Some((fa_path, fam_a)) = renamed_font(&dir, "verdana-clone", "verdana.ttf", "StkDup")
    else {
        eprintln!("SKIP：本机没有 verdana.ttf 可改名");
        std::fs::remove_dir_all(&dir).ok();
        if require_libass() {
            panic!("STK_REQUIRE_LIBASS=1，但造不出同名双份样本");
        }
        return;
    };
    let Some((mb_path, fam_b)) = renamed_font(&dir, "marlett-clone", "marlett.ttf", "StkDup")
    else {
        eprintln!("SKIP：本机没有 marlett.ttf 可改名");
        std::fs::remove_dir_all(&dir).ok();
        if require_libass() {
            panic!("STK_REQUIRE_LIBASS=1，但造不出同名双份样本");
        }
        return;
    };
    // 前提检查：两份必须落在**同一个**家族名上，否则考的就不是"同名谁赢"
    assert_eq!(
        fam_a, fam_b,
        "两份改名字体没得到同一个家族名（{fam_a} vs {fam_b}），这个样本测不了同名裁决"
    );
    let a = std::fs::read(&fa_path).unwrap();
    let b = std::fs::read(&mb_path).unwrap();
    assert_ne!(a, b, "两份样本字节相同，那就没有\"谁赢\"可测");

    let ass = ass_with(&fam_a, "Hello");
    let draw = |p: &mut Provider| -> Vec<u8> {
        let s = p.load_script(ass.as_bytes()).unwrap();
        render_libass::render(&s, Time::from_millis(2000).unwrap(), Canvas::HD720)
            .unwrap()
            .frame
            .rgba
    };

    let mut only_a = need_libass!(Canvas::HD720);
    only_a.add_font(&fam_a, &a).unwrap();
    let pa = draw(&mut only_a);
    let mut only_b = need_libass!(Canvas::HD720);
    only_b.add_font(&fam_b, &b).unwrap();
    let pb = draw(&mut only_b);
    assert_ne!(
        pa, pb,
        "单独注入两份却画出同样的像素：这个名字根本没参与匹配，同名裁决测试无从谈起"
    );

    let mut a_first = need_libass!(Canvas::HD720);
    a_first.add_font(&fam_a, &a).unwrap();
    a_first.add_font(&fam_b, &b).unwrap();
    let pab = draw(&mut a_first);
    let mut b_first = need_libass!(Canvas::HD720);
    b_first.add_font(&fam_b, &b).unwrap();
    b_first.add_font(&fam_a, &a).unwrap();
    let pba = draw(&mut b_first);

    let who = |p: &[u8]| -> Option<&'static str> {
        if p == pa.as_slice() {
            Some("A=verdana")
        } else if p == pb.as_slice() {
            Some("B=marlett")
        } else {
            None
        }
    };
    let w_ab = who(&pab).unwrap_or_else(|| {
        panic!("同时注册两份时像素既不是 A 也不是 B：内存字体没参与匹配，或与回退混排了")
    });
    let w_ba = who(&pba).unwrap_or_else(|| {
        panic!("同时注册两份时像素既不是 A 也不是 B：内存字体没参与匹配，或与回退混排了")
    });
    eprintln!("顺序裁决实测：A 先注册 -> {w_ab}；B 先注册 -> {w_ba}");
    // 契约一：注册顺序不改变结果（这条若翻了，说明 libass 换了同名裁决规则，
    // 那么"哪批字节画了像素"就不再确定，inject_plan 与报告口径都得重新核对）
    assert_eq!(
        pab, pba,
        "注册顺序改变了像素（{w_ab} vs {w_ba}）：libass 的同名裁决方式变了，\
         原先\"顺序不是裁决依据\"的实测结论作废——先重新取证再改代码"
    );
    // 契约二：赢家是交出去的那批字节之一，绝不是悄悄回退到系统字体
    assert!(w_ab == w_ba, "两种顺序赢家不同：{w_ab} vs {w_ba}");
    std::fs::remove_dir_all(&dir).ok();
}
