//! 36 §1 第三层 Preset 在 CLI 上的行为。
//!
//! ## 这一组核的是什么
//!
//! 规格 §2 给了一份 TOML 示例，§3 要求未知字段保留，§6 给了校验顺序。
//! 这些是**给人看的**规则，所以核的方式也应当是"用户会怎么用"：
//! 拿规格里的例子存成文件、拿它跑一遍、看输出与退出码。
//!
//! 最重要的一条是 [`the_spec_example_survives_a_real_round_trip`]：
//! 规格里那个例子必须**原样可用**。如果它自己跑不起来，
//! 用户照着抄一次就失败，而他们会以为是自己的错。

use std::path::PathBuf;
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("preset-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(cli()).args(args).output().expect("跑 CLI");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code(),
    )
}

/// **规格 §2 的例子必须原样可用。**
///
/// 而且是走**用户的那条路**：`preset example > f.toml` 然后再 validate。
///
/// 第一版这里就出过事：`example` 除了样板还打了一份 JSON 信封，
/// 于是存出来的文件前半 TOML、后半 JSON，validate 报"第 20 行 key 没有值"。
/// 一个"给你样板"的命令存出来自己读不了。
#[test]
fn the_spec_example_survives_a_real_round_trip() {
    let dir = scratch("roundtrip");
    let f = dir.join("spec.toml");

    let (stdout, _e, code) = run(&["preset", "example"]);
    assert_eq!(code, Some(0), "example 应当成功");
    std::fs::write(&f, &stdout).unwrap();

    // 存出来的必须是**纯 TOML**：`schema = 1` 是第一个非空字符。
    assert!(
        stdout.trim_start().starts_with("schema = 1"),
        "样板的第一行应当是 schema：{stdout}"
    );
    assert!(
        !stdout.contains("\"ok\""),
        "样板里不能混进 JSON 信封 —— 那会让这个文件读不回来：{stdout}"
    );

    let (out, err, code) = run(&["preset", "validate", f.to_str().unwrap()]);
    assert_eq!(
        code,
        Some(0),
        "规格自己的例子必须通过校验。stdout={out} stderr={err}"
    );
    assert!(out.contains("\"outputs\": 2"), "应当报出 2 个目标：{out}");
    assert!(
        out.contains("\"problems\": []"),
        "规格例子不该有任何问题：{out}"
    );
}

/// `show` 把内容摊开给人看：每个目标的名字、格式、画布、后缀。
#[test]
fn show_reports_each_target_with_its_effective_suffix() {
    let dir = scratch("show");
    let f = dir.join("p.toml");
    let (sample, _, _) = run(&["preset", "example"]);
    std::fs::write(&f, sample).unwrap();

    let (out, _e, code) = run(&["preset", "show", f.to_str().unwrap()]);
    assert_eq!(code, Some(0));
    for want in [
        "\"1080p-ASS\"",
        "\"1080p-Basic-SUP\"",
        "\"suffix\": \".1080p\"",
        "\"suffix\": \".1080p.basic\"",
        "\"hdr_mode\": \"off\"",
        "\"renderer\": \"libass\"",
        "\"pgs_validation\": \"basic\"",
    ] {
        assert!(out.contains(want), "应当报出 {want}：{out}");
    }
}

/// 36 §3：未知字段**保留但不参与执行**，而且必须**说出来**。
///
/// 只"保留"不"说"会让用户以为它生效了 —— 那是最坏的一种沉默。
#[test]
fn unknown_fields_are_preserved_and_reported_as_not_executed() {
    let dir = scratch("unknown");
    let f = dir.join("u.toml");
    std::fs::write(
        &f,
        "schema = 1\nname = \"u\"\nfuture_knob = 42\n\n[[outputs]]\nname = \"o\"\nformat = \"ass\"\nsuffix = \".a\"\n",
    )
    .unwrap();

    let (out, _e, code) = run(&["preset", "show", f.to_str().unwrap()]);
    assert_eq!(code, Some(0), "陌生字段不该让命令失败");
    assert!(out.contains("future_knob"), "要列出保留的字段名：{out}");
    assert!(out.contains("不参与执行"), "要说清它不生效：{out}");
}

/// 36 §6 第 1 条：schema 高于本程序时**拒绝**，并给出可行的下一步。
#[test]
fn a_future_schema_is_refused_with_an_actionable_message() {
    let dir = scratch("future");
    let f = dir.join("f.toml");
    std::fs::write(
        &f,
        "schema = 99\n\n[[outputs]]\nname = \"o\"\nformat = \"ass\"\n",
    )
    .unwrap();

    let (out, err, code) = run(&["preset", "validate", f.to_str().unwrap()]);
    assert_ne!(code, Some(0), "未来版本必须失败");
    let all = format!("{out}{err}");
    assert!(all.contains("99"), "要说清版本号：{all}");
    assert!(
        all.contains("升级") || all.contains("不猜"),
        "要给出下一步：{all}"
    );
}

/// 后缀碰撞会让两个目标写出同一个文件 —— 必须在**执行前**就查到。
#[test]
fn suffix_collisions_fail_validation_with_the_names() {
    let dir = scratch("collide");
    let f = dir.join("c.toml");
    std::fs::write(
        &f,
        "schema = 1\n\n[[outputs]]\nname = \"a\"\nformat = \"ass\"\nsuffix = \".out\"\n\n[[outputs]]\nname = \"b\"\nformat = \"pgs\"\nsuffix = \".out\"\n",
    )
    .unwrap();

    let (out, err, code) = run(&["preset", "validate", f.to_str().unwrap()]);
    assert_ne!(code, Some(0), "碰撞必须失败");
    let all = format!("{out}{err}");
    assert!(all.contains(".out"), "要说清是哪个后缀：{all}");
    assert!(
        all.contains("\"a\"") && all.contains("\"b\""),
        "要点名：{all}"
    );
}

/// 读不到的文件：退出码非零，而且错误里要有**路径**。
#[test]
fn a_missing_file_fails_with_the_path_in_the_message() {
    let dir = scratch("missing");
    let f = dir.join("nope.toml");
    let (out, err, code) = run(&["preset", "validate", f.to_str().unwrap()]);
    assert_ne!(code, Some(0));
    let all = format!("{out}{err}");
    assert!(all.contains("nope.toml"), "要说清是哪个文件：{all}");
}

/// 语法错要报 TOML 自己带位置的诊断，而不是我们猜的一句话。
#[test]
fn a_syntax_error_reports_the_position() {
    let dir = scratch("syntax");
    let f = dir.join("bad.toml");
    std::fs::write(&f, "schema = 1\n[[outputs]\nname = \"o\"\n").unwrap();
    let (out, err, code) = run(&["preset", "validate", f.to_str().unwrap()]);
    assert_ne!(code, Some(0));
    let all = format!("{out}{err}");
    assert!(
        all.contains("line") || all.contains("列") || all.contains("column"),
        "要指出位置，便于用户直接跳过去：{all}"
    );
}

/// 子命令拼错时要给出**能用的**选项，而不是只说"不认识"。
#[test]
fn a_wrong_subcommand_lists_the_valid_ones() {
    let (out, err, code) = run(&["preset", "wat"]);
    assert_ne!(code, Some(0));
    let all = format!("{out}{err}");
    for want in ["show", "validate", "example"] {
        assert!(all.contains(want), "要列出 {want}：{all}");
    }
}
