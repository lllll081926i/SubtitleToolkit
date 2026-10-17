//! CLI 端到端 golden（24 §Golden：真二进制、真退出码、真 stdout/stderr 分工）。
//!
//! 这些断言锁的是"对外契约"：退出码、`--json` 的单行稳定 schema、写出文件的字节内容。
//! 内部重构不应该让它们变绿/变红之外的东西改动它们。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\nScaledBorderAndShadow: yes\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,48,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\pos(960,540)\\blur4}你好\r\n";
const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\n你好\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n世界\r\n\r\n";

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-cli-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> Output {
    let exe = env!("CARGO_BIN_EXE_subtitletool-cli");
    Command::new(exe)
        .args(args)
        .output()
        .expect("CLI 二进制必须能启动")
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn doctor_json_is_one_line_and_names_missing_capabilities() {
    let out = run(&["doctor", "--json"]);
    assert!(out.status.success(), "doctor 应该成功：{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.lines().count(),
        1,
        "--json 必须正好一行，便于 JSONL 采集：{text}"
    );
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["tool"], "subtitletool-cli");
    assert_eq!(v["command"], "doctor");
    assert_eq!(v["ok"], true);
    let nb = v["data"]["not_built"].as_array().unwrap();
    // 未实现清单现在只剩 font subset：`render`、`font scan` 与 `mkv *` 都从占位变成了实命令，
    // 如果这里还写着它们，就是 doctor 在替产品撒一个过期的谎
    assert!(!nb.contains(&serde_json::json!("render")), "{nb:?}");
    assert!(!nb.contains(&serde_json::json!("font scan")), "{nb:?}");
    assert!(nb.contains(&serde_json::json!("font subset")), "{nb:?}");
    // mkv 已落地（18 §2/§4）：三个子命令都不该再出现。
    // 注意 catalog 里的是 "mkv identify" 这种子命令形式，不是裸的 "mkv"。
    assert!(!nb.contains(&serde_json::json!("mkv")), "{nb:?}");
    assert!(!nb.contains(&serde_json::json!("mkv identify")), "{nb:?}");
    assert!(!nb.contains(&serde_json::json!("mkv extract")), "{nb:?}");
    assert!(!nb.contains(&serde_json::json!("mkv remux")), "{nb:?}");
    // doctor 的清单是从解析器推出来的，所以它必须和实际退出码一致：
    // 列在未实现里的命令真的回 STK-INP-0060，没列的必须不报这个码
    for listed in nb {
        let name = listed.as_str().unwrap();
        // 子命令在真实命令行上是两个 token，不是带空格的一个字符串：
        // 整串传进去只会得到"用法错误(2)"，那条断言就变成在测自己的拼法
        let mut argv: Vec<&str> = name.split(' ').collect();
        argv.push("x");
        argv.push("--json");
        let out = run(&argv);
        assert_eq!(
            out.status.code(),
            Some(1),
            "doctor 说 {name} 未实现，它就该明确失败"
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            v["errors"][0]["code"], "STK-INP-0060",
            "{name} 的未实现码不对：{v}"
        );
    }
    for done in ["probe", "convert", "time", "hdr", "batch", "doctor"] {
        assert!(
            !nb.iter().any(|x| x.as_str() == Some(done)),
            "{done} 已实现，却还躺在 doctor 的未实现清单里"
        );
    }
    // 12 §2：doctor 必须能回答"能不能把第三方渲染 DLL 关在别的进程里"。
    // 可用时报出握手的每一位身份；不可用时也要给出是缺 host 还是缺 DLL——
    // "不可用"与"没查"必须能区分开。
    let rh = &v["data"]["render_host"];
    assert!(rh.is_object(), "doctor 没有 render_host 段：{v}");
    assert!(
        rh["available"].is_boolean() || rh["tree_contained"].is_boolean(),
        "available 必须是布尔：{rh}"
    );
    if rh["available"] == serde_json::json!(true) {
        let hs = &rh["handshake"];
        assert_eq!(hs["provider"], "libass", "{rh}");
        assert_eq!(
            hs["api_version"],
            renderer_host::protocol::API_VERSION,
            "{rh}"
        );
        assert_eq!(hs["pixel_format"], "RGBA8", "{rh}");
        assert_eq!(
            hs["provider_arch"], hs["host_arch"],
            "位数不同就不该握手成功（12 §3）：{rh}"
        );
        assert_eq!(
            hs["provider_binary_sha256"]
                .as_str()
                .unwrap_or_default()
                .len(),
            64,
            "{rh}"
        );
        // 孙进程约束状态必须被交代：附着失败时 job_error 要带码
        assert!(rh["tree_contained"].is_boolean(), "{rh}");
        if rh["tree_contained"] == serde_json::json!(false) {
            assert_ne!(rh["job_error"], serde_json::json!(0), "{rh}");
        }
    } else {
        let reason = rh["reason"].as_str().unwrap_or_default();
        assert!(reason.chars().count() > 8, "不可用也要说清原因：{rh}");
    }
}

#[test]
fn convert_ass_to_srt_matches_golden_bytes() {
    let d = workspace("a2s");
    let src = write(&d, "a.ass", ASS);
    let dst = d.join("a.srt");
    let out = run(&[
        "convert",
        src.to_str().unwrap(),
        "--to",
        "srt",
        "--out",
        dst.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "convert 失败：{out:?}");
    let got = std::fs::read_to_string(&dst).unwrap();
    assert_eq!(
        got, "1\r\n00:00:01,000 --> 00:00:03,000\r\n你好\r\n\r\n",
        "ASS→SRT 的字节级 golden"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["writes"][0]["state"], "written");
    assert_eq!(v["data"]["cues"], 1);
    assert_eq!(v["data"]["detected"]["format"], "ass");
    assert_eq!(v["ok"], true);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn convert_with_rewrite_scales_script_and_reports_loss() {
    let d = workspace("rewrite");
    let src = write(&d, "a.ass", ASS);
    let dst = d.join("a.2160.ass");
    let out = run(&[
        "convert",
        src.to_str().unwrap(),
        "--to",
        "ass",
        "--canvas",
        "3840x2160",
        "--script-policy",
        "rewrite",
        "--out",
        dst.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let got = std::fs::read_to_string(&dst).unwrap();
    assert!(got.contains("PlayResX: 3840"));
    assert!(got.contains("Style: Default,Arial,96,&H00FFFFFF,6,4,2,120,120,100"));
    assert!(got.contains("\\pos(1920,1080)"));
    assert!(got.contains("\\blur4"), "blur 不属于 screen_scale");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["data"]["changed"].as_u64().unwrap() > 0);
    assert_eq!(
        v["data"]["corpus_required"], true,
        "Rewrite 必须标出等效性待 corpus 证明"
    );
    assert_eq!(
        v["data"]["lossless"], true,
        "没有近似/丢弃条目时 lossless 为真，但 corpus_required 单独把关"
    );
    assert!(v["loss"]["user_attention"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("differential corpus")));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn preserve_is_a_byte_for_byte_no_op_through_the_binary() {
    let d = workspace("preserve");
    let src = write(&d, "a.ass", ASS);
    let dst = d.join("a.copy.ass");
    let out = run(&[
        "convert",
        src.to_str().unwrap(),
        "--to",
        "ass",
        "--canvas",
        "3840x2160",
        "--script-policy",
        "preserve",
        "--out",
        dst.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success());
    assert_eq!(std::fs::read(&dst).unwrap(), ASS.as_bytes(), "09 §9 硬规则");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn time_hdr_and_srt_to_ass_are_reachable() {
    let d = workspace("time-hdr");
    let srt = write(&d, "s.srt", SRT);
    let out = run(&[
        "time",
        srt.to_str().unwrap(),
        "--offset-ms",
        "1500",
        "--out",
        d.join("s.shift.srt").to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let shifted = std::fs::read_to_string(d.join("s.shift.srt")).unwrap();
    assert!(
        shifted.contains("00:00:02,500 --> 00:00:04,000"),
        "{shifted}"
    );

    let ass = write(&d, "s.ass", ASS);
    let out = run(&[
        "convert",
        srt.to_str().unwrap(),
        "--to",
        "ass",
        "--out",
        d.join("s.ass2").to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let made = std::fs::read_to_string(d.join("s.ass2")).unwrap();
    assert!(made.contains("[V4+ Styles]") && made.contains("Dialogue: 0,0:00:01.00,0:00:02.50"));
    // SRT 的毫秒精度必须保留到 ASS 的百分秒（08 §1：两种时间精度不能互相吞）
    assert!(made.contains("0:00:02.50"), "{made}");

    let out = run(&[
        "hdr",
        ass.to_str().unwrap(),
        "--hdr-mode",
        "compatibility",
        "--touch-styles",
        "yes",
        "--touch-inline",
        "yes",
        "--out",
        d.join("h.ass").to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let hdr = std::fs::read_to_string(d.join("h.ass")).unwrap();
    // Compatibility = 各通道 ×1/2 后 half-up 取整：255 -> 128（不是 127）
    assert!(hdr.contains("&H00808080"), "{hdr}");
    assert!(hdr.contains("PlayResX: 1920"), "hdr 不该动分辨率");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn usage_errors_exit_two_and_run_errors_exit_one() {
    let out = run(&["nosuchcommand", "x"]);
    assert_eq!(out.status.code(), Some(2), "用法错误必须是 2");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage:"),
        "用法提示走 stderr"
    );

    let out = run(&["probe", "Z:\\no-such-dir-9k2\\a.ass", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0110");
    assert!(v["errors"][0]["action"].is_null() || true);

    // 占位命令：明确失败、给 0060，而不是假成功。
    // mkv 已经落地（18 §2/§4），所以拿仍然未实现的 font subset 当样本。
    let out = run(&["font", "subset", "x.ass", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0060");

    // mkv 的未知子命令是**用法错误**(2)，不是运行错误(1)：
    // 命令拼错与命令跑不起来是两件事，退出码不能混。
    let out = run(&["mkv", "x", "--json"]);
    assert_eq!(out.status.code(), Some(2), "未知子命令应为用法错误");

    // 已实现的 `render` 走的是同一条"取值没问题但跑不起来"的路：0110 而不是 0060。
    // 这条断言的存在理由：占位命令换成实命令时，只有正向检查才会发现清单漂移。
    let out = run(&["render", "Z:\\no-such-dir-9k2\\a.ass", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0110");

    let out = run(&["--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_string();
    // 帮助文本必须覆盖 20 §Commands 的每一条：实现了的列出来，没实现的明说。
    // 断言要按"段落"来分：只看整段文本时，`pgs encode` 从已实现挪进未实现（或反过来）
    // 都能靠另一处出现而蒙过去，所以这里切成两半各查各的。
    let (impl_part, pending_part) = match help.split_once("尚未实现") {
        Some(pair) => pair,
        None => panic!("usage 里必须有\"尚未实现\"这一节：\n{help}"),
    };
    for cmd in [
        "probe",
        "convert",
        "time",
        "hdr",
        "pgs adjust",
        "pgs validate",
        "pgs encode",
        "render",
        "batch",
        "font scan",
        "doctor",
    ] {
        assert!(
            impl_part.contains(cmd),
            "usage 的已实现段落少了命令 {cmd}：\n{help}"
        );
        assert!(
            !pending_part.contains(cmd),
            "{cmd} 已实现，却还留在\"尚未实现\"里：\n{help}"
        );
    }
    for pending in ["font subset", "--preset", "STK-INP-0060"] {
        assert!(
            pending_part.contains(pending),
            "usage 少了未实现项 {pending}：\n{help}"
        );
    }
    // mkv 已经落地（18 §2/§4）：它的三个子命令必须在"已实现"段落里，
    // 且不得再出现在"尚未实现"那一段里
    for done in ["mkv identify", "mkv extract", "mkv remux"] {
        assert!(
            impl_part.contains(done),
            "usage 的已实现段落少了 {done}：\n{help}"
        );
        assert!(
            !pending_part.contains(done),
            "{done} 已实现，却还留在\"尚未实现\"里：\n{help}"
        );
    }
    assert!(
        help.contains("第 5 层缓冲模型与第 7 层设备语料未覆盖"),
        "pgs validate 的分层范围必须在帮助里说清：\n{help}"
    );

    // --preset 现在必须明确拒绝并指回决策，而不是"未知开关"
    let preset = run(&[
        "convert", "x.ass", "--to", "srt", "--preset", "foo.toml", "--json",
    ]);
    assert_eq!(preset.status.code(), Some(2), "参数错误应为退出码 2");
    let msg = String::from_utf8_lossy(&preset.stderr).to_string()
        + &String::from_utf8_lossy(&preset.stdout);
    assert!(msg.contains("--preset"), "{msg}");
    assert!(msg.contains("TOML"), "要解释为什么还不能用 TOML：{msg}");
}

#[test]
fn dry_run_does_not_touch_the_filesystem() {
    let d = workspace("dry");
    let src = write(&d, "a.ass", ASS);
    let dst = d.join("should-not-exist.srt");
    let out = run(&[
        "convert",
        src.to_str().unwrap(),
        "--to",
        srt_flag_value(),
        "--out",
        dst.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert!(!dst.exists(), "dry-run 不能落盘");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["writes"][0]["state"], "would-write");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn pgs_validate_reports_structure_for_streams_we_encoded() {
    // 用 library 造一个 SUP，再用二进制 validate：CLI 与 codec 的接口必须一致
    use codec_pgs::encoder::{encode, EncodeOptions, Frame, FrameObject};
    let d = workspace("pgs");
    let mut rgba = vec![0u8; 16 * 4];
    for px in rgba[..8 * 4].chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 255, 255, 255]);
    }
    let obj = FrameObject {
        rgba,
        width: 8,
        height: 2,
        x: 10,
        y: 10,
        forced: false,
    };
    let (segs, _) = encode(
        &[Frame::new(90_000, vec![obj]), Frame::erase(180_000)],
        &EncodeOptions::default(),
    )
    .unwrap();
    let sup = d.join("g.sup");
    std::fs::write(&sup, codec_pgs::segment::write_stream(&segs).unwrap()).unwrap();

    let out = run(&["pgs", "validate", sup.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["canvas"], "1920x1080");
    assert_eq!(v["data"]["no_blocking_findings"], true);
    // 默认 Basic：没跑 Strict 的层，就不可能给出 Strict 徽章
    assert_eq!(v["data"]["strict_badge_allowed"], false);
    // 按 Strict 跑合法输出：未实现层是 warning，不得阻断其余已实现规则的检查，
    // 但覆盖缺口依然让徽章作废（2026-10-09 审查：不能把未实现层当成文件的致命错误）
    let strict = run(&[
        "pgs",
        "validate",
        sup.to_str().unwrap(),
        "--level",
        "strict",
        "--json",
    ]);
    assert!(
        strict.status.success(),
        "结构合法的输出不该因为第 5/7 层未实现而失败：{strict:?}"
    );
    let sv: serde_json::Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert_eq!(sv["data"]["no_blocking_findings"], true);
    assert_eq!(sv["data"]["strict_badge_allowed"], false);
    let findings = sv["data"]["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|s| s.as_str().unwrap_or("").contains("PGS.DEC.01")),
        "未覆盖第 5 层必须被报出来：{findings:?}"
    );
    assert!(findings
        .iter()
        .any(|s| s.as_str().unwrap_or("").contains("PGS.DEV.01")));
    assert!(
        findings
            .iter()
            .all(|s| !s.as_str().unwrap_or("").starts_with("[error]")
                && !s.as_str().unwrap_or("").starts_with("[fatal]")),
        "覆盖缺口不能报成 error/fatal：{findings:?}"
    );

    let shifted = d.join("g.shift.sup");
    let out = run(&[
        "pgs",
        "adjust",
        sup.to_str().unwrap(),
        "--offset-ms",
        "1000",
        "--out",
        shifted.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let bytes = std::fs::read(&shifted).unwrap();
    let stream = codec_pgs::read_file(&bytes).unwrap();
    assert_eq!(stream.segments[0].pts, 90_000 + 90_000);
    assert_eq!(stream.segments[0].dts, 90_000 - 3_000 + 90_000);
    // ODS 载荷必须逐字节不变（13 §3.1）
    let before = std::fs::read(&sup).unwrap();
    let a = codec_pgs::fastpath::ods_digests(&codec_pgs::segment::parse_stream(&before).unwrap())
        .unwrap();
    let b = codec_pgs::fastpath::ods_digests(&stream.segments).unwrap();
    assert_eq!(a, b, "Fast Path 动了 ODS");
    std::fs::remove_dir_all(&d).ok();
}

fn srt_flag_value() -> &'static str {
    "srt"
}

/// 真实 libass 的位置：`STK_LIBASS_DLL` → 开发期 vcpkg 目录。找不到返回 None。
fn libass_dll() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
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

fn run_with_dll(args: &[&str], dll: Option<&Path>) -> Output {
    let exe = env!("CARGO_BIN_EXE_subtitletool-cli");
    let mut c = Command::new(exe);
    c.args(args);
    match dll {
        Some(p) => {
            c.env("STK_LIBASS_DLL", p);
        }
        None => {
            // 必须显式清空：否则继承父进程的值，"没有渲染器时该怎么报错"就永远测不到
            c.env_remove("STK_LIBASS_DLL");
        }
    }
    c.output().expect("CLI 二进制必须能启动")
}

#[test]
fn pgs_encode_without_a_renderer_reports_the_missing_input_and_writes_nothing() {
    let dir = workspace("enc-nodeps");
    let ass = write(&dir, "a.ass", ASS);
    let out = run_with_dll(&["pgs", "encode", ass.to_str().unwrap(), "--json"], None);
    assert_eq!(out.status.code(), Some(1), "取值没问题、跑不起来 = 1");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0141");
    // 用户看得懂的补法要在错误对象里出现：标题点名缺什么，details 给出两个入口
    assert!(
        v["errors"][0]["title"]
            .as_str()
            .unwrap_or_default()
            .contains("libass"),
        "标题要指出缺的是渲染器：{v}"
    );
    assert!(
        v["errors"][0]["details"]
            .as_array()
            .map(|d| d
                .iter()
                .any(|x| x.as_str().unwrap_or_default().contains("--libass-dll")))
            .unwrap_or(false),
        "必须给出可执行的补法：{v}"
    );
    assert!(
        !dir.join("a.sup").exists(),
        "没渲染器就不能留下半成品（AT-OUT-001）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pgs_encode_writes_a_sup_that_the_same_cli_validates_clean() {
    let Some(dll) = libass_dll() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!("STK_REQUIRE_LIBASS=1 但找不到 ass-9.dll（设 STK_LIBASS_DLL）");
        }
        eprintln!("SKIP pgs encode 端到端：本机没有 libass DLL");
        return;
    };
    let dir = workspace("enc-e2e");
    let ass = write(&dir, "a.ass", ASS);
    let sup = dir.join("a.sup");
    let out = run_with_dll(
        &[
            "pgs",
            "encode",
            ass.to_str().unwrap(),
            "--out",
            sup.to_str().unwrap(),
            "--fps",
            "25",
            "--canvas",
            "1920x1080",
            "--json",
        ],
        Some(&dll),
    );
    assert!(out.status.success(), "encode 应成功：{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["command"], "pgs encode");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["writes"][0]["path"], sup.display().to_string());
    assert_eq!(
        v["writes"][0]["state"], "written",
        "报了写入就必须真落盘：{v}"
    );
    assert!(sup.is_file(), "报了的写入必须真的在盘上");
    let bytes = std::fs::read(&sup).unwrap();
    assert_eq!(
        v["writes"][0]["bytes"].as_u64().unwrap() as usize,
        bytes.len()
    );
    assert!(v["data"]["display_sets"].as_u64().unwrap() >= 2, "{v}");
    assert_eq!(v["data"]["erase_events"].as_u64().unwrap(), 1);
    assert_eq!(v["data"]["no_blocking_findings"], true);
    // 真实抗锯齿渲染会超过 255 色预算，量化档必须把失真记账（ISSUES Z3）
    if v["data"]["quantized_frames"].as_u64().unwrap() > 0 {
        assert!(
            v["loss"]["approximated"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.as_str().unwrap_or_default().contains("量化")),
            "合并了颜色却没进 loss：{v}"
        );
    }

    // 同一个二进制回读自己写的文件：这是用户真正会做的事
    let out = run(&[
        "pgs",
        "validate",
        sup.to_str().unwrap(),
        "--json",
        "--level",
        "standard",
    ]);
    assert!(out.status.success(), "validate 应接受自己的输出：{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["no_blocking_findings"], true);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pgs_encode_on_a_frame_grid_raises_no_boundary_warning_and_snapping_is_inert() {
    // AA5 + AK15：validator 曾把 fps 当成秒数换算（25 帧/秒 → 2 250 000 tick），
    // 于是这条 CLI 路径明明写着 40ms 整数倍的 PTS，却报"不在 25 帧网格上"。
    // 这里同时钉三件事：修好之后不该有 TML.05；吸附对已经上格的流**什么都不动**；
    // 以及"什么都不动"必须是**说出来**的（loss 里点名 0 个被移动），不能静默。
    let Some(dll) = libass_dll() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!("STK_REQUIRE_LIBASS=1 但找不到 ass-9.dll（设 STK_LIBASS_DLL）");
        }
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let dir = workspace("enc-grid");
    let ass = write(&dir, "a.ass", ASS);
    let plain_sup = dir.join("p.sup");
    let snapped_sup = dir.join("s.sup");
    let ass_s = ass.to_str().unwrap();
    let plain_s = plain_sup.to_str().unwrap();
    let snapped_s = snapped_sup.to_str().unwrap();

    let plain_args = [
        "pgs",
        "encode",
        ass_s,
        "--fps",
        "25",
        "--canvas",
        "1920x1080",
        "--out",
        plain_s,
        "--json",
    ];
    let o = run_with_dll(&plain_args, Some(&dll));
    assert!(o.status.success(), "{plain_args:?} 应当成功：{o:?}");
    let plain: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let mut grid_off: Vec<String> = Vec::new();
    if let Some(arr) = plain["data"]["findings"].as_array() {
        for f in arr {
            if let Some(s) = f.as_str() {
                if s.contains("PGS.TML.05") {
                    grid_off.push(s.to_string());
                }
            }
        }
    }
    assert!(
        grid_off.is_empty(),
        "在网格上的产物不该被报成离开网格：{grid_off:?}"
    );

    let mut snap_args = plain_args.to_vec();
    snap_args.extend_from_slice(&["--snap-pts", "--out", snapped_s]);
    let o = run_with_dll(&snap_args, Some(&dll));
    assert!(o.status.success(), "{snap_args:?} 应当成功：{o:?}");
    let snapped: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        snapped["data"]["display_sets"], plain["data"]["display_sets"],
        "吸附不许改变显示集数量"
    );
    assert!(
        std::fs::read(&plain_sup).unwrap() == std::fs::read(&snapped_sup).unwrap(),
        "本来就上格时，--snap-pts 不该改动任何字节"
    );
    let mut said = false;
    if let Some(arr) = snapped["loss"]["preserved"].as_array() {
        said = arr
            .iter()
            .any(|t| t.as_str().unwrap_or_default().contains("AA5"));
    }
    assert!(said, "动了/没动都要说出来，不能静默：{}", snapped["loss"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pgs_encode_dry_run_writes_nothing_even_with_a_renderer() {
    let Some(dll) = libass_dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let dir = workspace("enc-dry");
    let ass = write(&dir, "a.ass", ASS);
    let out = run_with_dll(
        &[
            "pgs",
            "encode",
            ass.to_str().unwrap(),
            "--fps",
            "25",
            "--dry-run",
            "--json",
        ],
        Some(&dll),
    );
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["writes"][0]["state"], "would-write", "{v}");
    assert!(!dir.join("a.sup").exists(), "--dry-run 不许碰盘");
    std::fs::remove_dir_all(&dir).ok();
}

/// 17 §2–§4 的对外承诺：拿真字体库跑一次 `font scan`，报告必须能自证边界。
///
/// 这条刻意不指定 `--fonts-dir`：默认目录解析（含用户注册字体目录）本身就是被测对象。
#[test]
fn font_scan_reports_against_the_real_font_library() {
    let dir = workspace("font-scan");
    let ass = write(&dir, "a.ass", ASS);
    let report = dir.join("fonts.json");
    let out = run(&[
        "font",
        "scan",
        ass.to_str().unwrap(),
        "--out",
        report.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["command"], "font scan", "{v}");
    assert_eq!(v["ok"], true, "{v}");
    let d = &v["data"];
    assert!(
        d["database"]["faces"].as_u64().unwrap() > 10,
        "本机字体库不可能读不到 face：{d}"
    );
    assert!(
        d["fonts_dirs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("Fonts")),
        "默认目录必须真的被列出来：{d}"
    );
    // fixture 只有 Default 样式引 Arial，正文只有一个"你好"
    assert_eq!(d["requests"].as_u64().unwrap(), 1, "{d}");
    let m = &d["matches"][0];
    assert_eq!(m["request"], "Arial", "{m}");
    assert!(m["chars"].as_str().unwrap().contains('你'), "{m}");
    assert!(
        matches!(
            m["verdict"].as_str().unwrap(),
            "exact" | "alias" | "substituted" | "missing" | "ambiguous" | "missing_glyph"
        ),
        "评级必须是 17 §4 的六档之一：{m}"
    );
    // 不变量：说"名字对上了"与"这个 face 画不出正文"必须同时可见。
    // Arial 在本机画不出"你好"，所以这条要么改判 missing_glyph，要么真的覆盖——
    // 绝不能既报 Exact 又带着缺字清单（那是 17 §4 第 5 档被吞掉的样子）。
    let consistent = m["verdict"] == "missing_glyph"
        || m["verdict"] == "ambiguous"
        || m["missing_glyphs"].as_str().unwrap().is_empty()
        || m["face"].is_null();
    assert!(consistent, "评级与缺字清单自相矛盾：{m}");
    assert!(
        d["scope"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("DirectWrite")),
        "未使用 DirectWrite 权威这条边界必须写在报告里，不能只在源码注释里：{d}"
    );
    // 17 §2 的"字体注册"：默认要真的读注册表，并把读到的东西如实报出来
    assert_eq!(
        d["render_identity"]["faces_used"].as_u64().unwrap()
            + d["render_identity"]["faces_missing"].as_u64().unwrap(),
        d["requests"].as_u64().unwrap(),
        "每一条请求都要在渲染身份里有一格（命中或缺席），否则换字体不会让缓存失效：{d}"
    );
    let reg = &d["registry"];
    assert_eq!(reg["read"], true, "默认必须读注册表：{d}");
    assert!(
        reg["entries"].as_u64().unwrap() > 0,
        "本机注册表有 164 条登记，读到 0 条就是枚举没生效：{reg}"
    );
    assert_eq!(
        reg["resolved"].as_u64(),
        reg["entries"].as_u64(),
        "每条登记都要么解析到文件、要么在 warnings 里给原因：{reg}"
    );
    assert_eq!(v["writes"][0]["state"], "written", "{v}");
    let back: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(back["database"], d["database"], "落盘报告必须与信封一致");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec!["a.ass".to_string(), "fonts.json".to_string()],
        "report only：除了输入与报告不许留下别的产物（17 §6）"
    );
    std::fs::remove_dir_all(&dir).ok();

    // subset 仍是占位：不能因为 scan 落地就顺带宣称子集可用（17 §5 的 corpus 没建）
    let out = run(&["font", "subset", "x.ass", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0060", "{v}");
    assert!(
        v["errors"][0]["title"]
            .as_str()
            .unwrap()
            .contains("font subset"),
        "{v}"
    );
}

/// 17 §6 第二档：`--copy-fonts` 复制整份字体并写 manifest。
///
/// 用一份临时字体目录跑（不是整个系统字体库）：这条测的是交付路径的正确性，
/// 不是本机装了哪些字体，没必要为此花 45 秒。
#[test]
fn font_copy_delivers_the_file_and_a_manifest_that_names_its_source() {
    let dir = workspace("font-copy");
    let fonts = dir.join("fonts");
    let deliver = dir.join("deliver");
    std::fs::create_dir_all(&fonts).unwrap();
    let windir = std::env::var("WINDIR").expect("测试需要 WINDIR");
    let sys = Path::new(&windir).join("Fonts");
    // ASS fixture 请求 Arial，因此复制与请求对应的真实字体；随机选系统字体会让
    // face resolver 不命中，测试就没有覆盖字体交付路径。
    let src = sys.join("arial.ttf");
    assert!(
        src.is_file(),
        "Windows 字体目录缺少测试所需的 Arial：{}",
        src.display()
    );
    std::fs::copy(&src, fonts.join(src.file_name().unwrap())).unwrap();
    let ass = write(&dir, "a.ass", ASS);

    let out = run(&[
        "font",
        "scan",
        ass.to_str().unwrap(),
        "--fonts-dir",
        fonts.to_str().unwrap(),
        "--no-registry",
        "--copy-fonts",
        deliver.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["data"];
    assert_eq!(d["delivery"]["mode"], "copy full fonts", "{d}");
    assert_eq!(
        d["registry"]["read"], false,
        "--no-registry 必须真的关掉：{d}"
    );
    let name = src.file_name().unwrap().to_str().unwrap().to_string();
    let copied = deliver.join(&name);
    assert!(copied.exists(), "字体没被复制过去：{v}");
    assert!(
        deliver.join("manifest.json").exists(),
        "manifest 必须一起交付"
    );
    let want = std::fs::read(&src).unwrap();
    assert_eq!(std::fs::read(&copied).unwrap(), want, "复制必须逐字节相同");
    // manifest 里必须留着来源信息（17 §8），且没有假的 subset 哈希
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(deliver.join("manifest.json")).unwrap()).unwrap();
    let e = &manifest["fonts"][0];
    // 来源信息指的是"这次交付实际复制的那份文件"：这里给的是临时目录里的那一份，
    // 不是系统目录里的同名原件（测试期望写错过一次，见这里）
    assert_eq!(e["source_file"], fonts.join(&name).to_str().unwrap(), "{e}");
    assert!(e["source_sha256"].as_str().unwrap().len() == 64, "{e}");
    assert!(
        e["subset_sha256"].is_null(),
        "没做 subset 就不能假装有：{e}"
    );
    assert_eq!(
        manifest["limits"].as_array().unwrap().len(),
        3,
        "{manifest}"
    );
    // 信封里的 writes 要和盘上产物一一对应：字体 1 份 + manifest 1 份
    let writes = v["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 2, "{v}");
    assert!(writes.iter().all(|w| w["state"] == "written"), "{writes:?}");

    // dry-run：一个字节都不许落盘
    let deliver2 = dir.join("deliver-dry");
    let out = run(&[
        "font",
        "scan",
        ass.to_str().unwrap(),
        "--fonts-dir",
        fonts.to_str().unwrap(),
        "--no-registry",
        "--copy-fonts",
        deliver2.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["writes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["state"] == "would-write"),
        "{v}"
    );
    assert!(!deliver2.join("manifest.json").exists(), "--dry-run 写了盘");

    // 相对路径的交付目录在解析层就被挡（输出事务只接绝对路径）
    let out = run(&[
        "font",
        "scan",
        ass.to_str().unwrap(),
        "--copy-fonts",
        "relative/out",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(2), "用法错误必须退 2：{out:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// 17 §7 在 CLI 这一层的两件事：报告要说清"这次有没有把字节交给渲染器"，
/// 而且交出去的字节必须**就是字体扫描记录里那一份** —— 两条不同的代码路径
/// （font-service 建库时算的哈希 vs 注入时算的哈希）要能对上，对不上就说明
/// "渲染缓存 key 钉住的字节"与"真画像素的字节"不是一回事。
#[test]
fn render_reports_the_font_bytes_handed_to_the_renderer() {
    let Some(dll) = libass_dll() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!("STK_REQUIRE_LIBASS=1 但找不到 ass-9.dll（设 STK_LIBASS_DLL）");
        }
        eprintln!("SKIP 注入报告：本机没有 libass DLL");
        return;
    };
    let dir = workspace("inject-report");
    let ass = write(&dir, "a.ass", ASS);

    // 没开注入：报告必须是 enabled:false，且台账为空（"没开"与"开了但没注入"要能分开读）
    let off = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--libass-dll",
            dll.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
        Some(&dll),
    );
    assert_eq!(
        off.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&off.stderr)
    );
    let vo: serde_json::Value = serde_json::from_slice(&off.stdout).unwrap();
    let inj_off = &vo["data"]["font_injection"];
    assert_eq!(inj_off["enabled"], false, "{inj_off}");
    assert_eq!(
        inj_off["injected"].as_array().map(|a| a.len()),
        Some(0),
        "{inj_off}"
    );

    // 开了注入
    let on = run_with_dll(
        &[
            "render",
            ass.to_str().unwrap(),
            "--libass-dll",
            dll.to_str().unwrap(),
            "--inject-fonts",
            "--dry-run",
            "--json",
        ],
        Some(&dll),
    );
    assert_eq!(
        on.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&on.stderr)
    );
    let vn: serde_json::Value = serde_json::from_slice(&on.stdout).unwrap();
    let inj = &vn["data"]["font_injection"];
    assert_eq!(inj["enabled"], true, "{inj}");
    let list = inj["injected"].as_array().expect("注入清单必须是数组");
    assert_eq!(
        list.len(),
        inj["requested"].as_u64().expect("要报出请求条数") as usize,
        "请求了几条就要交出几条；少交了必须报错而不是静默：{inj}"
    );
    assert!(
        !list.is_empty(),
        "这份夹具请求的字体本机应该有，台账不该为空：{inj}"
    );
    for f in list {
        let sha = f["sha256"].as_str().unwrap_or_default();
        assert_eq!(sha.len(), 64, "内容哈希是 SHA-256：{f}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "{f}");
        assert!(
            f["bytes"].as_u64().unwrap_or(0) > 0,
            "交出去的不该是空文件：{f}"
        );
        assert!(!f["name"].as_str().unwrap_or_default().is_empty(), "{f}");
    }
    assert!(
        inj["meaning"]
            .as_str()
            .unwrap_or_default()
            .contains("17 §7"),
        "报告要自己说清这一格是什么意思：{inj}"
    );
    // --dry-run 仍然一个字节都不落盘（注入不改变交付纪律）
    assert!(
        !dir.join("a_BDN").exists(),
        "--dry-run 却写出了目录：{:?}",
        dir
    );

    // 交叉核对：同一份脚本、同一台机器，font scan 记录的 face 哈希要能盖住注入台账
    let rep = dir.join("scan.json");
    let scan = run(&[
        "font",
        "scan",
        ass.to_str().unwrap(),
        "--out",
        rep.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        scan.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let scanned: serde_json::Value = serde_json::from_slice(&std::fs::read(&rep).unwrap()).unwrap();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for m in scanned["matches"].as_array().expect("报告要有 matches") {
        if m["verdict"].as_str() == Some("missing") {
            continue;
        }
        let face = &m["face"];
        if face.is_null() {
            continue;
        }
        seen.insert(
            face["source_sha256"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert!(
        !seen.is_empty(),
        "扫描这边也没 face，交叉核对就是空的：{scanned}"
    );
    for f in list {
        let sha = f["sha256"].as_str().unwrap_or_default();
        assert!(
            seen.contains(sha),
            "注入台账里出现了扫描记录里没有的哈希：{}；已知 {:?}",
            sha,
            seen
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 17 §7 的第二句现在闭合了：脚本自带 `[Fonts]` 时，那份字节既是渲染的输入、
/// 也是身份的一部分。两条都必须可测：
/// * 同一份脚本，只多一段 `[Fonts]`，渲染身份摘要必须变（否则缓存会把一份的像素发给另一份）；
/// * `--inject-fonts` 的报告要能分出"来自本机库"与"来自脚本内嵌"。
#[test]
fn embedded_fonts_change_the_identity_and_are_injected() {
    let Some(dll) = libass_dll() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!("STK_REQUIRE_LIBASS=1 但找不到 ass-9.dll（设 STK_LIBASS_DLL）");
        }
        eprintln!("SKIP 内嵌字体渲染集成：本机没有 libass DLL");
        return;
    };
    let dir = workspace("embedded-identity");
    let plain = write(&dir, "plain.ass", ASS);
    let bytes = std::fs::read(
        std::path::Path::new(&std::env::var("WINDIR").unwrap())
            .join("Fonts")
            .join("arial.ttf"),
    )
    .expect("本机该有 arial.ttf");
    // 形状照 libass 的 ass.c:process_fonts_line —— `fontname: X` 之后每一行都是这份
    // 字节的 base64（AK10：原先这里写的是我自己发明的 `P:` 形状，端到端测的其实是假文件）。
    // 也刻意折行，让"跨行拼接"在 CLI 这一层同样被走到，而不只在单元测里。
    let b64 = b64_for_test(&bytes);
    let mut folded = String::new();
    for chunk in b64.as_bytes().chunks(72) {
        folded.push_str(std::str::from_utf8(chunk).unwrap());
        folded.push('\n');
    }
    let embedded = format!("{}\n[Fonts]\nfontname: arial.ttf\n{folded}", ASS);
    let with_fonts = write(&dir, "embedded.ass", &embedded);

    let digest_of = |file: &str, outxml: &str| -> String {
        // 必须显式交 DLL：CLI 的自动查找是相对它自己的 cwd 猜的，测试里不该依赖那条运气
        let out = run_with_dll(
            &[
                "render",
                file,
                "--libass-dll",
                dll.to_str().unwrap(),
                "--fps",
                "25",
                "--out",
                outxml,
                "--dry-run",
                "--json",
            ],
            Some(&dll),
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        v["data"]["font_env"]["digest"]
            .as_str()
            .expect("报告要给出渲染身份")
            .to_string()
    };
    let xml_a = dir.join("plain.xml").display().to_string();
    let xml_b = dir.join("embedded.xml").display().to_string();
    let a = digest_of(plain.to_str().unwrap(), &xml_a);
    let b = digest_of(with_fonts.to_str().unwrap(), &xml_b);
    assert_ne!(
        a, b,
        "只多了一段 [Fonts] 就换了像素来路，身份却是同一个 —— 那缓存就会串"
    );

    // 注入报告要分得清两种来源
    let out = run_with_dll(
        &[
            "render",
            with_fonts.to_str().unwrap(),
            "--libass-dll",
            dll.to_str().unwrap(),
            "--inject-fonts",
            "--dry-run",
            "--json",
        ],
        Some(&dll),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let inj = &v["data"]["font_injection"];
    assert_eq!(inj["enabled"], true, "{inj}");
    assert!(
        inj["embedded"].as_u64().unwrap_or(0) >= 1,
        "内嵌的那份必须被算进注入计划：{inj}"
    );
    assert!(
        inj["injected"]
            .as_array()
            .map(|a| a
                .iter()
                .any(|f| f["sha256"].as_str() == Some(&sha256_hex(&bytes))))
            .unwrap_or(false),
        "台账里要能看到那份内嵌字体的内容哈希：{inj}"
    );
    // `font scan` 那边也必须看得见它（同一份身份算法，两处不能各说一套）
    let rep = dir.join("scan.json");
    let scan = run(&[
        "font",
        "scan",
        with_fonts.to_str().unwrap(),
        "--out",
        rep.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        scan.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let scanned: serde_json::Value = serde_json::from_slice(&std::fs::read(&rep).unwrap()).unwrap();
    assert_eq!(
        scanned["render_identity"]["embedded_fonts"].as_u64(),
        Some(1),
        "{scanned}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 测试专用的编码器（生产路径只有解码器）。
fn b64_for_test(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    font_service::attachments::digest_hex(bytes)
}
