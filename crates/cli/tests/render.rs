//! 14 §ASS→BDN 的端到端契约（AT-BDN-001、AT-OUT-001、20 §Commands）。
//!
//! 这里刻意不用 `codec-bdn` 的校验器来当唯一裁判：测试自己读 PNG 的 IHDR、自己数
//! `<Event>`。否则"writer 与 validator 同一双手写错同一个地方"这类缺陷就看不见
//! （ISSUES M2 的教训：自家 writer + 自家 reader 的往返不算证据）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, Outline, Shadow, Alignment, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,64,&H00FFFFFF,&H00000000,2,0,2,20,20,60\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,first line\r\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,second line\r\n";

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-render-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 真实 libass：`STK_LIBASS_DLL` → 仓库旁边的 vcpkg 目录。
/// 门阶用 `STK_REQUIRE_LIBASS=1` 把"没跑"变成失败（ISSUES Z4）。
fn libass_dll() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let guess = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(|p| p.parent())
        .map(|root| {
            root.join("tools")
                .join("vcpkg")
                .join("installed")
                .join("x64-windows")
                .join("bin")
                .join("ass-9.dll")
        })
        .unwrap_or_default();
    guess.is_file().then_some(guess)
}

macro_rules! need_libass {
    () => {
        match libass_dll() {
            Some(p) => p,
            None => {
                if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
                    panic!("STK_REQUIRE_LIBASS=1，但本机没有可用的 ass-9.dll（设 STK_LIBASS_DLL）");
                }
                eprintln!("SKIP：本机没有 libass DLL");
                return;
            }
        }
    };
}

fn run_with_dll(args: &[&str], dll: Option<&Path>) -> Output {
    let exe = env!("CARGO_BIN_EXE_subtitletool-cli");
    let mut c = Command::new(exe);
    c.args(args);
    match dll {
        Some(p) => {
            c.env("STK_LIBASS_DLL", p);
        }
        None => {
            // 必须显式清空：否则继承父进程的值，"没有渲染器时该怎么报错"永远测不到
            c.env_remove("STK_LIBASS_DLL");
        }
    }
    c.output().expect("CLI 二进制必须能启动")
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout 不是单行 JSON：{e}：{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// 测试自己读的 BDN 结构：事件行 + 声明尺寸/位置/文件名。
struct Declared {
    file: String,
    width: u32,
    height: u32,
    in_tc: String,
    out_tc: String,
}

fn tag_value(chunk: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let a = chunk.find(&open)? + open.len();
    let b = chunk[a..].find(&close)? + a;
    Some(chunk[a..b].to_string())
}

fn attr(chunk: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let a = chunk.find(&needle)? + needle.len();
    let b = chunk[a..].find('"')? + a;
    Some(chunk[a..b].to_string())
}

/// 从 XML 里把每个事件的声明读出来（不经过 codec-bdn 的 parser）。
fn declared(xml: &str) -> Vec<Declared> {
    let mut out = Vec::new();
    for ev in xml.split("<Event ").skip(1) {
        let ev = ev.split("</Event>").next().unwrap_or(ev);
        let head = ev.split('>').next().unwrap_or(ev);
        out.push(Declared {
            file: tag_value(ev, "FileName").expect("事件必须有 FileName"),
            width: tag_value(ev, "Width")
                .expect("事件必须有 Width")
                .parse()
                .expect("Width 是十进制"),
            height: tag_value(ev, "Height")
                .expect("事件必须有 Height")
                .parse()
                .expect("Height 是十进制"),
            in_tc: attr(head, "InTC").expect("事件必须有 InTC"),
            out_tc: attr(head, "OutTC").expect("事件必须有 OutTC"),
        });
    }
    out
}

/// PNG IHDR 的真实尺寸：签名 8 字节 + 长度 4 + "IHDR" 4，其后 4+4 个大端。
fn ihdr_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(
        &png[0..8],
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    );
    assert_eq!(&png[12..16], b"IHDR");
    let be = |o: usize| u32::from_be_bytes([png[o], png[o + 1], png[o + 2], png[o + 3]]);
    (be(16), be(20))
}

fn tc_is_frames_and_seconds(tc: &str) -> bool {
    let parts: Vec<&str> = tc.split(':').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit()))
}

#[test]
fn render_writes_a_bdn_dir_whose_declared_sizes_are_the_png_sizes() {
    let dll = need_libass!();
    let dir = workspace("bdn");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let xml = dir.join("bdn").join("a.xml");
    let out = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--canvas",
            "1920x1080",
            "--out",
            xml.to_str().unwrap(),
            "--json",
        ],
        Some(&dll),
    );
    assert!(
        out.status.success(),
        "render 应成功：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["command"], "render");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["to"], "bdn");
    assert_eq!(v["data"]["events"].as_u64().unwrap(), 2, "{v}");
    assert_eq!(v["data"]["rendered_frames"].as_u64().unwrap(), 2);

    // 报的每条写入都必须真在盘上，且 XML 是最后一条
    let writes = v["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 3, "{v}");
    for w in writes {
        assert_eq!(w["state"], "written", "报了写入就必须落盘：{v}");
        assert!(Path::new(w["path"].as_str().unwrap()).is_file(), "{v}");
    }
    assert_eq!(writes[2]["path"], xml.display().to_string());

    let text = std::fs::read_to_string(&xml).unwrap();
    assert_eq!(
        writes[2]["bytes"].as_u64().unwrap() as usize,
        text.len(),
        "报告的字节数要与盘上一致"
    );
    // 画布声明就是完整 16:9（09 §1）
    assert_eq!(tag_value(&text, "Width").unwrap(), "1920");
    assert_eq!(tag_value(&text, "Height").unwrap(), "1080");

    let events = declared(&text);
    assert_eq!(events.len(), 2);
    // 单一时间源：1s/3s 的 Dialogue 落到 25fps 帧网格上就是 01:00 / 03:00
    assert_eq!(events[0].in_tc, "00:00:01:00", "{text}");
    assert_eq!(events[1].in_tc, "00:00:03:00", "{text}");
    assert_eq!(events[0].out_tc, "00:00:02:00", "{text}");
    assert_eq!(events[1].out_tc, "00:00:04:00", "{text}");
    for ev in &events {
        assert!(tc_is_frames_and_seconds(&ev.in_tc), "{}", ev.in_tc);
        assert!(tc_is_frames_and_seconds(&ev.out_tc), "{}", ev.out_tc);
        // AT-BDN-001：XML 声明的尺寸 == PNG IHDR 的实际尺寸
        let png = std::fs::read(dir.join("bdn").join(&ev.file))
            .unwrap_or_else(|e| panic!("{} 没落盘：{e}", ev.file));
        let (w, h) = ihdr_size(&png);
        assert_eq!((w, h), (ev.width, ev.height), "{} 声明与实际不符", ev.file);
        assert!(w > 0 && h > 0 && w <= 1920 && h <= 1080, "越界：{w}x{h}");
        assert_ne!(
            (w, h),
            (1920, 1080),
            "tight 裁剪应该交出条状位图，而不是整幅画布"
        );
    }
    // 文件名连续编号，不留孤儿也不留空洞
    let names: Vec<&str> = events.iter().map(|e| e.file.as_str()).collect();
    assert_eq!(names, vec!["0000.png", "0001.png"], "{text}");
    let pngs_on_disk: Vec<String> = std::fs::read_dir(dir.join("bdn"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".png"))
        .collect();
    assert_eq!(pngs_on_disk.len(), 2, "盘上有孤儿 PNG：{pngs_on_disk:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn render_dry_run_writes_nothing_at_all() {
    let dll = need_libass!();
    let dir = workspace("dry");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--out",
            dir.join("bdn").join("a.xml").to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
        Some(&dll),
    );
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["ok"], true, "{v}");
    for w in v["writes"].as_array().unwrap() {
        assert_eq!(w["state"], "would-write", "{v}");
    }
    assert!(!dir.join("bdn").exists(), "--dry-run 不许建目录");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn render_skips_an_existing_target_when_told_to() {
    let dll = need_libass!();
    let dir = workspace("overwrite");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let xml = dir.join("bdn").join("a.xml");
    let first = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--out",
            xml.to_str().unwrap(),
            "--json",
        ],
        Some(&dll),
    );
    assert!(first.status.success(), "{first:?}");
    std::fs::write(&xml, "SENTINEL").unwrap();
    let second = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--out",
            xml.to_str().unwrap(),
            "--overwrite",
            "skip",
            "--json",
        ],
        Some(&dll),
    );
    assert!(second.status.success(), "{second:?}");
    let v = json(&second);
    let writes = v["writes"].as_array().unwrap();
    assert_eq!(writes.last().unwrap()["state"], "skipped", "{v}");
    assert_eq!(
        std::fs::read_to_string(&xml).unwrap(),
        "SENTINEL",
        "skip 策略下动了原文件（15 §6、AT-OUT-001）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn min_axis_that_filters_everything_reports_an_empty_bdn_instead_of_fake_success() {
    let dll = need_libass!();
    let dir = workspace("minaxis");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--min-axis",
            "9999",
            "--out",
            dir.join("bdn").join("a.xml").to_str().unwrap(),
            "--json",
        ],
        Some(&dll),
    );
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["data"]["events"].as_u64().unwrap(), 0);
    assert_eq!(v["data"]["rendered_frames"].as_u64().unwrap(), 2);
    assert_eq!(
        v["data"]["pngs"].as_u64().unwrap(),
        0,
        "过滤掉的对象不能留下孤儿 PNG"
    );
    // 只有 XML 一条写入，且盘上没有 PNG
    let writes = v["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 1, "{v}");
    assert_eq!(writes[0]["state"], "written");
    let text = std::fs::read_to_string(dir.join("bdn").join("a.xml")).unwrap();
    assert!(!text.contains("<Event "), "{text}");
    assert_eq!(declared(&text).len(), 0);
    let attention: Vec<&str> = v["loss"]["user_attention"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap_or_default())
        .collect();
    assert!(
        attention.iter().any(|t| t.contains("没有任何事件")),
        "空 BDN 必须如实说明：{v}"
    );
    assert!(
        v["loss"]["dropped"].as_array().unwrap().iter().any(|t| {
            t.as_str()
                .unwrap_or_default()
                .contains("小于 min_axis 9999 的小对象")
        }),
        "过滤记账要进 loss：{v}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn render_without_a_renderer_fails_with_the_missing_input_and_writes_nothing() {
    let dir = workspace("nodeps");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = run_with_dll(&["render", ass.to_str().unwrap(), "--json"], None);
    assert_eq!(out.status.code(), Some(1), "取值没问题、跑不起来 = 1");
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0141");
    assert!(
        v["errors"][0]["details"]
            .as_array()
            .map(|d| d
                .iter()
                .any(|x| x.as_str().unwrap_or_default().contains("--libass-dll")))
            .unwrap_or(false),
        "必须给出可执行的补法：{v}"
    );
    assert!(!dir.join("a_BDN").exists(), "没有渲染器就不能留半成品");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn render_rejects_a_non_widescreen_canvas_as_a_usage_error() {
    // 09 §1：BDN 输出画布永远是完整 16:9。这里退出码必须是 2（用法错误），
    // 而不是 1——用户参数写错与跑不起来是两回事（20 §Exit codes）。
    let dir = workspace("canvas");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--canvas",
            "1024x768",
            "--json",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(2));
    // 用法错误走 stderr 文本，不开 JSON 信封：信封是给"跑起来了并且有结论"的运行用的
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "用法错误不该有信封"
    );
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(text.contains("16:9"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn render_default_output_lives_next_to_the_input() {
    let dll = need_libass!();
    let dir = workspace("default-out");
    let ass = dir.join("my-sub.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = run_with_dll(
        &["render", ass.to_str().unwrap(), "--fps", "25", "--json"],
        Some(&dll),
    );
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    let xml = Path::new(v["data"]["xml"].as_str().unwrap());
    assert_eq!(xml.file_name().unwrap(), "my-sub.xml");
    assert_eq!(xml.parent().unwrap().file_name().unwrap(), "my-sub_BDN");
    assert!(xml.is_file(), "默认路径报了写入却没落盘：{v}");
    std::fs::remove_dir_all(&dir).ok();
}
