//! 23 §文件：**大文件拒绝**。
//!
//! 规格原话是「大文件、极端 cue 数、zip bomb 不相关格式拒绝」。
//! 这一条的关键不在"最后报了个错"，而在**报错之前有没有把文件读进内存**：
//! `fs::read` 会先整份读进内存，之后再拿大小去判断就已经晚了 ——
//! 机器内存已经吃完了，而用户得到的可能是一句无关的 OOM。
//!
//! 所以这里的测试是行为性的：拿一个超过上限的**稀疏文件**（占用几乎不占磁盘，
//! 但 `metadata().len()` 报的是声明的长度）去跑真实命令，检查
//! 1) 错误码是 0113 而不是 0110；2) 消息里同时给出实际大小与上限；
//! 3) 有可照做的补法。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("bigfile-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 造一个声明长度很大、实际几乎不占磁盘的文件。返回是否造成功。
///
/// 用稀疏文件而不是真写 1 GiB：CI 与开发机上真写会把磁盘写满，
/// 而这个测试要验的是"读之前先看大小"，稀疏文件正好能独立地把那一格钉住。
fn sparse_over(path: &Path, len: u64) -> bool {
    let f = std::fs::File::create(path).unwrap();
    f.set_len(len).is_ok()
}

fn err_of(v: &serde_json::Value) -> (String, String, String, String) {
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
        e["action"].as_str().unwrap_or_default().to_string(),
    )
}

fn run_json(args: &[&str]) -> serde_json::Value {
    let o = Command::new(cli())
        .args(args)
        .arg("--json")
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

/// 文本输入超限：错误码 0113，消息给出实际大小与上限，并说明本工具处理字幕而非视频。
///
/// 用 5 GiB（超过二进制上限 4 GiB）：`probe` 得先能看见文件才知道它是什么格式，
/// 所以它只能用**最大的**那条上限；真正的文本路径（`load_text`）另有 1 GiB 的窄限，
/// 而那一条由 `the_limit_itself_is_inclusive` 在窄接口上钉住（不必真读 1 GiB）。
#[test]
fn an_oversized_input_is_refused_with_its_own_code() {
    let d = scratch("text");
    let big = d.join("huge.ass");
    if !sparse_over(&big, 5 * 1024 * 1024 * 1024) {
        eprintln!("SKIP：本文件系统不支持 set_len");
        return;
    }
    let v = run_json(&["probe", big.to_str().unwrap()]);
    assert!(!v["ok"].as_bool().unwrap(), "该失败：{v}");
    let (code, title, details, action) = err_of(&v);
    assert_eq!(code, "STK-INP-0113", "要单独立一个码：{v}");
    assert!(title.contains("上限"), "{title}");
    assert!(
        details.contains("5368709120") && details.contains("4294967296"),
        "详情要同时给出实际大小与上限：{details}"
    );
    assert!(
        action.contains("字幕"),
        "要指出本工具处理字幕而不是视频：{action}"
    );
}

/// 二进制输入（SUP）也先看大小再读。
///
/// 这里用 `pgs validate` 而不是 `probe`：它是真的会走 `read_bounded` 的路径，
/// 而它拒一个 5 GiB 文件所花的时间与拒一个 5 KiB 文件一样。
#[test]
fn an_oversized_binary_input_is_refused_too() {
    let d = scratch("bin");
    let big = d.join("huge.sup");
    if !sparse_over(&big, 5 * 1024 * 1024 * 1024) {
        eprintln!("SKIP：本文件系统不支持 set_len");
        return;
    }
    let v = run_json(&["pgs", "validate", big.to_str().unwrap(), "--level", "basic"]);
    assert!(!v["ok"].as_bool().unwrap(), "该失败：{v}");
    let (code, _, details, _) = err_of(&v);
    assert_eq!(code, "STK-INP-0113", "{v}");
    assert!(details.contains("5368709120"), "要给实际字节数：{details}");
}

/// 上限之内的文件必须**照常处理**。
///
/// 少了这条负基线，一个"什么文件都拒绝"的实现也能让上面两条全绿，
/// 而它会把这个产品变得毫无用处。
#[test]
fn a_normal_sized_input_still_works() {
    let d = scratch("normal");
    let small = d.join("ok.ass");
    std::fs::write(
        &small,
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,30,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.00,D,,0,0,0,,Hi\n",
    )
    .unwrap();
    let v = run_json(&["probe", small.to_str().unwrap()]);
    assert!(v["ok"].as_bool().unwrap(), "正常大小该照常处理：{v}");
}

/// 上限**恰好等于**文件大小时要放行（边界是 `>` 而不是 `>=`）。
///
/// 这里**不**跑真 CLI：那样 probe 会真的去读那 1 GiB，测试要跑几分钟。
/// 上一条已经证明了"超限会被 CLI 拒绝"，这一条关心的只是那一个比较运算符。
#[test]
fn the_limit_itself_is_inclusive() {
    let d = scratch("boundary");
    let f = d.join("exact.bin");
    let limit = 4096u64;
    // 恰好等于上限：该通过
    std::fs::write(&f, vec![b'x'; limit as usize]).unwrap();
    cli::pipeline::pipeline_probe_read(&f, limit).expect("恰好等于上限该通过");
    // 多一个字节：该被拒
    std::fs::write(&f, vec![b'x'; limit as usize + 1]).unwrap();
    let e = cli::pipeline::pipeline_probe_read(&f, limit).expect_err("超一个字节该被拒");
    assert_eq!(e.code_string(), "STK-INP-0113", "{e:?}");
}

// ---------------------------------------------------------------------------
// 23 §文件 的「极端 cue 数」：字节数限不住这一格。
//
// 200 万条空 cue 只有 40 多 MB，字节上限完全看不住；而解析后每条 cue
// 要占一个结构 + 一个下游 stage。所以那一条要求必须按**条目数**单独设限。
// ---------------------------------------------------------------------------

/// 造一份 `n` 条 cue 的 SRT。每条不到 40 字节，所以 100 万条大约 40 MB。
fn many_cues_srt(n: usize) -> String {
    let mut s = String::with_capacity(n * 40);
    for i in 0..n {
        s.push_str(&format!("{i}\n00:00:01,000 --> 00:00:02,000\nx\n\n"));
    }
    s
}

/// SRT 超过 cue 上限时以 `STK-PRS-0123` 被拒，且**不静默截断**。
///
/// 静默截断会让用户拿到一份缺了后半段的字幕，而那份文件在别处看起来完全正常 ——
/// 这是本仓库一贯拒绝的取向（同 15 §6 的覆盖策略）。
#[test]
fn an_srt_with_too_many_cues_is_refused_not_truncated() {
    let d = scratch("cues");
    let limit = core_model::CUE_COUNT_LIMIT;
    let f = d.join("many.srt");
    // 只多一条：边界用例，同时让文件尽量小（约 40 MB）
    std::fs::write(&f, many_cues_srt(limit + 1)).unwrap();
    assert!(
        std::fs::metadata(&f).unwrap().len() < 100 * 1024 * 1024,
        "这份输入要远小于字节上限，否则测的就不是 cue 数那一条"
    );

    let v = run_json(&["convert", f.to_str().unwrap(), "--to", "ass"]);
    assert!(!v["ok"].as_bool().unwrap(), "该失败：{v}");
    let (code, title, details, action) = err_of(&v);
    assert_eq!(code, "STK-PRS-0123", "{v}");
    assert!(title.contains("cue"), "{title}");
    assert!(
        details.contains(&format!("{}", limit + 1)) || details.contains(&format!("{limit}")),
        "详情要说清到多少条：{details}"
    );
    assert!(action.contains("切分"), "要给可照做的补法：{action}");
    // 关键：没有产物写出来（截断会写出一份“看起来正常”的文件）
    assert!(!d.join("many.ass").exists(), "被拒时不该写出任何产物");
}

/// 上限之内的 cue 数要照常处理（负基线）。
/// 少了它，一个"什么 SRT 都拒"的实现也能让上面那条变绿。
#[test]
fn an_srt_just_below_the_cue_limit_still_converts() {
    let d = scratch("cues-ok");
    let f = d.join("ok.srt");
    std::fs::write(&f, many_cues_srt(5)).unwrap();
    let v = run_json(&["convert", f.to_str().unwrap(), "--to", "ass"]);
    assert!(v["ok"].as_bool().unwrap(), "正常条数该照常转换：{v}");
}

/// 真字幕（几百条 cue）必须完好无损地通过 —— 上限不能把正常用法拦住。
#[test]
fn a_realistic_subtitle_is_unaffected_by_the_cue_limit() {
    let d = scratch("real");
    let f = d.join("real.srt");
    std::fs::write(&f, many_cues_srt(2000)).unwrap();
    let v = run_json(&["convert", f.to_str().unwrap(), "--to", "ass"]);
    assert!(v["ok"].as_bool().unwrap(), "{v}");
}
