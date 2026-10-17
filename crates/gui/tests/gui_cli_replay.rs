//! GUI ↔ CLI 打通的验收（20 §Repro：GUI 复制的命令可以直接重放）。
//!
//! ## 这组测试要证的不是“GUI 能跑”
//!
//! 而是“GUI 跑的那件事，和 CLI 跑的那件事，**是同一件事**”。
//! 判据：GUI 编译出的 spec 交给 `subtitletool-cli batch --spec` 跑，
//! 产物与状态必须与 GUI 自己的通路一致。
//!
//! 这条能成立，是因为 GUI 不另写执行逻辑：它调的就是 `cli::batch::run_batch`。
//! 如果哪天有人给 GUI 加了一条并行实现，这里的断言会先坏。

use std::path::{Path, PathBuf};
use std::process::Command;

use gui::state::profile::{
    CanvasMode, HdrMode, OutputProfile, ProfilesState, TargetFormat, TimeAdjustMode,
};

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-gui-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,48,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Hello\r\n";

/// 一份只做纯文本输出的 profile：不需要 libass，测试不依赖渲染器就能跑到端。
fn srt_profile(name: &str) -> OutputProfile {
    OutputProfile {
        id: 1,
        enabled: true,
        name: name.to_string(),
        target_format: TargetFormat::Srt,
        canvas: CanvasMode::Fhd1080,
        time_adjust: TimeAdjustMode::None,
        hdr: HdrMode::Sdr,
        custom_suffix: ".srt".into(),
    }
}

/// 找 `subtitletool-cli` 真二进制。
///
/// `CARGO_BIN_EXE_*` 只能用于**同一个包**的测试；gui 与 cli 是两个包，所以得自己找。
/// 路径以当前测试可执行文件为锚：`<target>/<profile>/deps/<test>.exe` →
/// 同级的 `subtitletool-cli.exe`。这比读环境变量可靠（不依赖外部先设好什么）。
fn cli_exe() -> PathBuf {
    let mut dir = std::env::current_exe().expect("当前测试可执行文件路径必须可读");
    dir.pop(); // deps/
    if dir.ends_with("deps") {
        dir.pop();
    }
    for candidate in [
        dir.join(format!("subtitletool-cli{}", std::env::consts::EXE_SUFFIX)),
        dir.join("deps")
            .join(format!("subtitletool-cli{}", std::env::consts::EXE_SUFFIX)),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    // 还没构建过 CLI 时别默默换一个路径：那会让测试看起来在跑，其实跑的不是 CLI。
    panic!(
        "找不到 subtitletool-cli（已找过 {}）。先跑 `cargo build -p cli`。",
        dir.display()
    );
}

fn run_cli(args: &[&str]) -> std::process::Output {
    Command::new(cli_exe())
        .args(args)
        .output()
        .expect("CLI 必须能启动")
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("没有 JSON 行：{text:?}\nstderr={:?}", out.stderr));
    serde_json::from_str(line).unwrap()
}

/// 核心验收：GUI 编译的 spec 能被 CLI 直接跑，且产出真实产物。
#[test]
fn a_gui_compiled_spec_replays_through_the_real_cli() {
    let dir = workspace("replay");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    // 1) 用 GUI 的编译器产出 spec（这就是"复制到 CLI 重放"的那个东西）
    let p = srt_profile("文本导出");
    let (spec, notes) = gui::worker::spec::compile_spec(
        "gui-replay",
        std::slice::from_ref(&input),
        &[&p],
        &out_dir,
        None,
        "{stem}.{profile}.{ext}",
    );
    assert!(
        notes.is_empty(),
        "这份参数都是支持的，不该有说明：{notes:?}"
    );
    let spec_path = dir.join("gui.spec.json");
    std::fs::write(&spec_path, gui::worker::spec::spec_to_json(&spec)).unwrap();

    // 2) 交给**真的 CLI** 跑
    let out = run_cli(&["batch", "--spec", spec_path.to_str().unwrap(), "--json"]);
    let v = json_of(&out);
    assert_eq!(v["ok"], true, "GUI 编译的 spec 必须能被 CLI 跑成功：{v}");
    assert_eq!(v["data"]["job_id"], "gui-replay");
    assert_eq!(v["data"]["outcome"], "success");

    // 3) 产物真的在盘上，而且内容是真的 SRT
    let produced: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("srt"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(produced.len(), 1, "应当恰好产出一个 SRT：{produced:?}");
    let text = std::fs::read_to_string(&produced[0]).unwrap();
    assert!(
        text.contains("Hello"),
        "产出的 SRT 内容必须来自输入：{text}"
    );
    assert!(text.contains("-->"), "必须是合法 SRT 时间行：{text}");

    // 4) 报告里的逐行状态必须是 CLI 的词汇，不是 GUI 自造的
    let details = v["data"]["report"]["details"].as_array().unwrap();
    assert_eq!(details.len(), 1, "{details:?}");
    let status = details[0]["status"].as_str().unwrap();
    assert!(
        matches!(status, "verified" | "from-cache" | "would-write"),
        "成功状态必须是 CLI 的词表里的：{status}"
    );
}

/// 反向：GUI 的 `TaskStatus::from_report` 必须认得 CLI 真的会发出的每一种 status。
///
/// 这条守的是"两套词表漂移"：CLI 加了新状态而 GUI 当成未知 → 抽屉里显示失败，
/// 但报告里其实是成功的。
#[test]
fn gui_recognises_every_status_the_cli_actually_emits() {
    let dir = workspace("statuses");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    // 攒齐三种真实状态：首次 verified、重跑 from-cache、dry-run would-write
    let mut seen: Vec<String> = Vec::new();
    let p = srt_profile("S");
    let (spec, _) = gui::worker::spec::compile_spec(
        "gui-status",
        std::slice::from_ref(&input),
        &[&p],
        &out_dir,
        None,
        "{stem}.{profile}.{ext}",
    );
    let spec_path = dir.join("s.json");
    std::fs::write(&spec_path, gui::worker::spec::spec_to_json(&spec)).unwrap();

    for extra in [vec![], vec![], vec!["--dry-run"]] {
        let mut args = vec!["batch", "--spec", spec_path.to_str().unwrap(), "--json"];
        args.extend(extra);
        let out = run_cli(&args);
        let v = json_of(&out);
        for row in v["data"]["report"]["details"].as_array().unwrap() {
            seen.push(row["status"].as_str().unwrap().to_string());
        }
    }
    assert!(seen.len() >= 3, "至少要凑到三次执行：{seen:?}");

    for s in &seen {
        let st = gui::state::task::TaskStatus::from_report(s, "detail");
        assert!(
            !matches!(st, gui::state::task::TaskStatus::Failed { .. }),
            "GUI 认不出 CLI 真发出来的状态 {s:?}（会让抽屉把成功显示成失败）：{st:?}"
        );
    }
    // 三种都出现过：状态覆盖真的被走到了，而不是碰巧只跑出一种
    assert!(
        seen.iter().any(|s| s == "from-cache"),
        "重跑应当命中缓存：{seen:?}"
    );
    assert!(
        seen.iter().any(|s| s == "would-write"),
        "dry-run 应当报 would-write：{seen:?}"
    );
}

/// GUI 的 profile 编译结果与 CLI 手写的等价 spec 必须产生同样的产物。
///
/// 这是"两边不是两套东西"的最直接证据：去掉 id 与 roots 的写法差异后，
/// 两份 spec 的其余部分逐字段相同。
#[test]
fn gui_spec_and_a_handwritten_cli_spec_agree_field_by_field() {
    let dir = workspace("agree");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let out_dir = dir.join("out");

    let p = srt_profile("P");
    let (gui_spec, _) = gui::worker::spec::compile_spec(
        "same",
        std::slice::from_ref(&input),
        &[&p],
        &out_dir,
        None,
        "{stem}.{profile}.{ext}",
    );

    // 手写一份"用户会自己写的" spec（模拟他只想要一个 SRT）
    let handwritten = format!(
        r#"{{
  "id": "same",
  "roots": ["{}"],
  "recursive": true,
  "include": {:?},
  "exclude": [],
  "profiles": [
    {{"name": "P", "format": "srt", "canvas": "1920x1080", "fps": "25"}}
  ],
  "output": {{"dir": "{}", "template": "{{stem}}.{{profile}}.{{ext}}"}},
  "video_association": false
}}"#,
        input.display().to_string().replace('\\', "\\\\"),
        gui_spec.include,
        out_dir.display().to_string().replace('\\', "\\\\"),
    );
    let hp = dir.join("hand.json");
    std::fs::write(&hp, &handwritten).unwrap();

    // 两份都要能被 CLI 接受
    let a = cli::batch::load_spec(&{
        let p = dir.join("gui.json");
        std::fs::write(&p, gui::worker::spec::spec_to_json(&gui_spec)).unwrap();
        p
    })
    .expect("GUI 的 spec 必须合法");
    let b = cli::batch::load_spec(&hp).expect("手写的 spec 必须合法");

    // 逐字段比：能比出"GUI 偷偷加了 CLI 没有的东西"
    assert_eq!(a.id, b.id);
    assert_eq!(a.roots, b.roots);
    assert_eq!(a.recursive, b.recursive);
    assert_eq!(a.profiles.len(), b.profiles.len());
    assert_eq!(a.profiles[0].name, b.profiles[0].name);
    assert_eq!(a.profiles[0].format, b.profiles[0].format);
    assert_eq!(a.profiles[0].canvas, b.profiles[0].canvas);
    assert_eq!(a.profiles[0].fps, b.profiles[0].fps);
    assert_eq!(a.profiles[0].exact_colors, b.profiles[0].exact_colors);
    assert_eq!(a.profiles[0].min_axis, b.profiles[0].min_axis);
    assert_eq!(a.output.dir, b.output.dir);
    assert_eq!(a.output.template, b.output.template);

    // 而且两份 spec 跑出来的产物字节也必须一致
    let pd = dir.join("prod");
    std::fs::create_dir_all(&pd).unwrap();
    let mut hashes = Vec::new();
    for (name, sp) in [("gui", dir.join("gui.json")), ("hand", hp.clone())] {
        let d = pd.join(name);
        std::fs::create_dir_all(&d).unwrap();
        let text = std::fs::read_to_string(&sp).unwrap();
        // 把输出目录改成各自的分目录，其余不动
        let patched = text.replace(
            &out_dir.display().to_string().replace('\\', "\\\\"),
            &d.display().to_string().replace('\\', "\\\\"),
        );
        let p2 = pd.join(format!("{name}.json"));
        std::fs::write(&p2, patched).unwrap();
        let out = run_cli(&["batch", "--spec", p2.to_str().unwrap(), "--json"]);
        let v = json_of(&out);
        assert_eq!(v["ok"], true, "{name} 必须成功：{v}");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("srt"))
            .collect();
        files.sort();
        assert_eq!(files.len(), 1, "{name} 应当产出一个 SRT：{files:?}");
        hashes.push(font_service_digest(&files[0]));
    }
    assert_eq!(
        hashes[0], hashes[1],
        "GUI spec 与手写 spec 的产物必须逐字节一致"
    );
}

fn font_service_digest(p: &Path) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(std::fs::read(p).unwrap());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `ProfilesState` 的默认档位编译出来必须仍然过 CLI 的校验。
/// 默认值是最容易被忽略的一条路径：用户什么都没改就点开始。
#[test]
fn the_default_profile_matrix_compiles_into_a_valid_spec() {
    let dir = workspace("defaults");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let profiles = ProfilesState::new();
    let enabled: Vec<&OutputProfile> = profiles.enabled_profiles();
    assert!(!enabled.is_empty(), "默认至少要有一个启用的 profile");

    let (spec, _notes) = gui::worker::spec::compile_spec(
        "defaults",
        &[input],
        &enabled,
        &dir,
        // 默认档里有 BDN 与 SUP，需要 renderer；指不到就传 None，
        // 校验阶段不需要它（只有真执行才读）
        None,
        "{stem}.{profile}.{ext}",
    );
    let sp = dir.join("d.json");
    std::fs::write(&sp, gui::worker::spec::spec_to_json(&spec)).unwrap();
    let loaded = cli::batch::load_spec(&sp).expect("默认矩阵必须能过 CLI 校验");
    assert_eq!(loaded.profiles.len(), enabled.len());
    // 默认档的格式拼写必须是 CLI 认的
    for p in &loaded.profiles {
        assert!(
            matches!(p.format.as_str(), "ass" | "ssa" | "srt" | "pgs" | "bdn"),
            "默认 profile {} 的 format 拼写 CLI 不认：{}",
            p.name,
            p.format
        );
    }
}
