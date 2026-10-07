//! pysubs2 语义交叉验证（AT-ASS-001 / 24 §QA 的"外部实现参与"）。
//!
//! 为什么还要再来一个第三方：`codec-ass` 的 parse→render 往返只证明"我们跟自己一致"。
//! 时间格式、字段顺序、默认值这类解释错误，只有自己两套代码都按同一种误解写才会全绿。
//! pysubs2（MIT）是独立实现，用它读我们的文件、也用它写的文件喂我们。
//!
//! 依赖 `python` + `pysubs2`；缺失时明确打印 SKIP（`STK_REQUIRE_PYSUBS2=1` 时视为失败）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codec_ass::ast::AssDocument;
use serde::{Deserialize, Serialize};

const FIXTURE: &str = "[Script Info]\n\
Title: cross-check\n\
ScriptType: v4.00+\n\
PlayResX: 1920\n\
PlayResY: 1080\n\
ScaledBorderAndShadow: yes\n\
WrapStyle: 0\n\
\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,50,50,60,1\n\
Style: Speaker,Noto Sans CJK SC,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,8,10,10,20,1\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\an5\\pos(960,540)}第一行 {\\i1}斜体{\\i0} 尾巴\n\
Dialogue: 1,0:00:04.01,0:00:06.00,Speaker,A,0,0,30,banner,两句\n第二行\n";

#[derive(Debug, Deserialize, Serialize)]
struct StyleModel {
    name: String,
    fontname: String,
    fontsize: String,
    alignment: String,
    margin_l: String,
    margin_r: String,
    margin_v: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct EventModel {
    start_ms: i64,
    end_ms: i64,
    layer: String,
    style: String,
    name: String,
    margin_l: String,
    margin_r: String,
    margin_v: String,
    effect: String,
    text: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Model {
    playres: Option<[u32; 2]>,
    sbs: Option<String>,
    title: Option<String>,
    styles: Vec<StyleModel>,
    events: Vec<EventModel>,
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("subs_pysubs2_check.py")
}

fn python() -> Option<String> {
    if let Ok(p) = std::env::var("STK_PYTHON") {
        return Some(p);
    }
    for cand in ["python", "python3", "py"] {
        let ok = Command::new(cand)
            .arg("-c")
            .arg("import pysubs2")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(cand.to_string());
        }
    }
    None
}

fn run(python: &str, verb: &str, dir: &Path) -> (bool, String) {
    let out = Command::new(python)
        .arg(script())
        .arg(verb)
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), text)
        }
        Err(e) => (false, format!("启动 {python} 失败：{e}")),
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("stk-pysubs-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ms(t: core_model::time::Time) -> i64 {
    let r = t.as_rational();
    // ASS 时间分辨率是厘秒，*1000 一定整除；不整除说明时间来源不是厘秒网格
    (r.num() * 1000) / r.den()
}

/// 从**我们的**解析结果构造语义模型（字段顺序与 Python 侧约定一致）。
fn our_model(doc: &AssDocument) -> Model {
    let info = |key: &str| doc.script_info(key).map(|v| v.trim().to_string());
    let playres = match (info("PlayResX"), info("PlayResY")) {
        (Some(x), Some(y)) => Some([x.parse().unwrap(), y.parse().unwrap()]),
        _ => None,
    };
    let mut styles: Vec<StyleModel> = Vec::new();
    for st in doc.styles() {
        let g = |k: &str| st.field(k).unwrap_or("").to_string();
        styles.push(StyleModel {
            name: st.name().unwrap_or("").to_string(),
            fontname: g("Fontname"),
            fontsize: g("Fontsize"),
            alignment: g("Alignment"),
            margin_l: g("MarginL"),
            margin_r: g("MarginR"),
            margin_v: g("MarginV"),
        });
    }
    styles.sort_by(|a, b| a.name.cmp(&b.name));
    let mut events = Vec::new();
    for d in doc.dialogues() {
        let text = d.text().to_string();
        // 多行 Dialogue 的续行会被我们的解析器合并成一条事件；这里按同一约定还原
        events.push(EventModel {
            start_ms: ms(d.start().unwrap()),
            end_ms: ms(d.end().unwrap()),
            layer: d.layer().unwrap_or("0").to_string(),
            style: d.style_name().unwrap_or("").to_string(),
            name: d.actor().unwrap_or("").to_string(),
            margin_l: d.margin_l().unwrap_or("0").to_string(),
            margin_r: d.margin_r().unwrap_or("0").to_string(),
            margin_v: d.margin_v().unwrap_or("0").to_string(),
            effect: d.effect().unwrap_or("").to_string(),
            text,
        });
    }
    Model {
        playres,
        sbs: info("ScaledBorderAndShadow").map(|v| v.to_lowercase()),
        title: info("Title"),
        styles,
        events,
    }
}

#[test]
fn pysubs2_reads_our_ass_and_we_read_pysubs2_ass() {
    let Some(python) = python() else {
        if std::env::var("STK_REQUIRE_PYSUBS2").is_ok() {
            panic!("STK_REQUIRE_PYSUBS2=1 但找不到带 pysubs2 的 python");
        }
        eprintln!("SKIP pysubs2 交叉验证：本机没有 python + pysubs2");
        return;
    };
    let dir = tmpdir("x");

    // 方向一：我们写 → pysubs2 读
    let doc = AssDocument::parse(FIXTURE);
    let rendered = doc.render();
    std::fs::write(dir.join("ours_a.ass"), rendered.as_bytes()).unwrap();
    let model = our_model(&doc);
    std::fs::write(
        dir.join("ours_a.json"),
        serde_json::to_vec_pretty(&model).unwrap(),
    )
    .unwrap();
    let (ok, log) = run(&python, "verify", &dir);
    assert!(ok, "pysubs2 复核我们的 ASS 失败：{log}");
    assert!(log.contains("VERIFY OK 1"), "没核对到文件：{log}");

    // 方向二：pysubs2 写 → 我们读
    let (ok, log) = run(&python, "emit", &dir);
    assert!(ok, "pysubs2 生成参考文件失败：{log}");
    let mut refs = 0usize;
    for name in ["ref_rich.ass", "ref_plain.ass"] {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let their_doc = AssDocument::parse(&text);
        let got = our_model(&their_doc);
        let want: Model =
            serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
        assert_eq!(got.playres, want.playres, "{name} PlayRes");
        assert_eq!(got.sbs, want.sbs, "{name} ScaledBorderAndShadow");
        assert_eq!(got.title, want.title, "{name} Title");
        assert_eq!(got.styles.len(), want.styles.len(), "{name} 样式数量");
        for (a, b) in got.styles.iter().zip(want.styles.iter()) {
            assert_eq!(
                (a.name.as_str(), a.fontname.as_str()),
                (b.name.as_str(), b.fontname.as_str()),
                "{name} 样式名/字体"
            );
            assert_eq!(a.fontsize, b.fontsize, "{name} {}.fontsize", a.name);
            assert_eq!(a.alignment, b.alignment, "{name} {}.alignment", a.name);
            assert_eq!(a.margin_l, b.margin_l, "{name} {}.margin_l", a.name);
            assert_eq!(a.margin_v, b.margin_v, "{name} {}.margin_v", a.name);
        }
        assert_eq!(got.events.len(), want.events.len(), "{name} 事件数");
        for (i, (a, b)) in got.events.iter().zip(want.events.iter()).enumerate() {
            assert_eq!(a.start_ms, b.start_ms, "{name} events[{i}].start_ms");
            assert_eq!(a.end_ms, b.end_ms, "{name} events[{i}].end_ms");
            assert_eq!(a.layer, b.layer, "{name} events[{i}].layer");
            assert_eq!(a.style, b.style, "{name} events[{i}].style");
            assert_eq!(a.name, b.name, "{name} events[{i}].name");
            assert_eq!(a.margin_v, b.margin_v, "{name} events[{i}].margin_v");
            assert_eq!(a.effect, b.effect, "{name} events[{i}].effect");
            assert_eq!(a.text, b.text, "{name} events[{i}].text");
        }
        refs += 1;
    }
    assert!(refs >= 2, "pysubs2 只产出 {refs} 个 ASS 参考文件");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn srt_semantics_match_pysubs2_too() {
    let Some(python) = python() else {
        if std::env::var("STK_REQUIRE_PYSUBS2").is_ok() {
            panic!("STK_REQUIRE_PYSUBS2=1 但找不到带 pysubs2 的 python");
        }
        eprintln!("SKIP srt 交叉验证：本机没有 python + pysubs2");
        return;
    };
    let dir = tmpdir("srt");
    let (ok, log) = run(&python, "emit", &dir);
    assert!(ok, "emit 失败：{log}");
    let path = dir.join("ref.srt");
    if !path.is_file() {
        eprintln!("SKIP：pysubs2 没产出 ref.srt");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let (sub, _loss) =
        codec_srt::parse(&bytes).unwrap_or_else(|e| panic!("读不回 pysubs2 写的 SRT：{e}"));
    let want: Model =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(
        sub.cues.len(),
        want.events.len(),
        "SRT 条目数不符：我们 {} vs pysubs2 {}",
        sub.cues.len(),
        want.events.len()
    );
    for (i, (ours, theirs)) in sub.cues.iter().zip(want.events.iter()).enumerate() {
        assert_eq!(ms(ours.start), theirs.start_ms, "srt cues[{i}].start");
        assert_eq!(ms(ours.end), theirs.end_ms, "srt cues[{i}].end");
        assert_eq!(ours.text, theirs.text, "srt cues[{i}].text");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
