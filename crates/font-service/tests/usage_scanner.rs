//! 17 §3 的用量扫描器契约。全部用内存里的 ASS 文本，不需要任何字体文件 ——
//! 这一层只回答"字幕引用了哪个 face、要画哪些字"。
//!
//! fixture 一律写成真实 ASS 的样子：override tag 必须在 `{...}` 块里。第一版我把
//! `\fn` 直接裸写在文本中（那不是合法 ASS），于是"扫描器没跟上新 face"看起来像实现错，
//! 其实是测试构造的语言不对（17 §3 要的正是按真语法的 state track）。

use font_service::{scan_text, FaceRequest};

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Alignment, MarginV, Bold, Italic\r\nStyle: Default,Arial,48,&H00FFFFFF,2,0,2,60,-1,0\r\nStyle: hei,SimHei,48,&H00FFFFFF,2,0,2,60,0,-1\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,粗体{\\b1}加粗{\\b0}收尾\r\nDialogue: 0,0:00:03.00,0:00:04.00,hei,,0,0,0,,{\\fnTimes New Roman}西文{\\r}中文第二行\r\n";

fn req<'a>(list: &'a [FaceRequest], family: &str) -> Option<&'a FaceRequest> {
    list.iter().find(|r| r.family == family)
}

/// 17 §3 的第一条：Style 字段按**动态 Format 行**解析。
/// Bold/Italic 在这里是第 9/10 列；按固定列号读会读到 Alignment/MarginV。
#[test]
fn style_fields_follow_the_dynamic_format_line() {
    let got = scan_text(ASS).unwrap();
    // req() 返回排序后的第一条（bold=false 在前），所以按字集找"正文那条"
    let arial = got
        .iter()
        .find(|r| r.family == "Arial" && r.chars.contains(&'粗'))
        .expect("Default style 的 Arial 必须被记一次");
    assert!(
        arial.bold,
        "Style 的 Bold=-1 让正文本来就是粗体，第一段的字该落在 bold 那条：{arial:?}"
    );
    assert!(!arial.italic);
    let hei = req(&got, "SimHei").expect("style hei 的 SimHei");
    assert!(!hei.bold);
    assert!(hei.italic, "Italic=-1 → 该 style 的基准是斜体：{hei:?}");
}

/// `\fn` 改变的是"后面这些字用哪个 face"，`\r` 再改回 style 的基准。
#[test]
fn inline_tags_move_the_current_face_forward() {
    let got = scan_text(ASS).unwrap();
    let times = req(&got, "Times New Roman").expect("\\fn 引入的 face 必须有用量记录");
    assert!(
        times.chars.contains(&'西') && times.chars.contains(&'文'),
        "字集没跟着 \\fn 走：{times:?}"
    );
    assert!(
        !times.chars.contains(&'中') && !times.chars.contains(&'第'),
        "\\r 之后的字仍算 Times 的，说明 reset 没生效：{times:?}"
    );
    assert!(
        times.sources.iter().any(|s| s.contains("\\fn")),
        "来源要指到具体哪一行的哪个 tag：{times:?}"
    );

    let hei = req(&got, "SimHei").unwrap();
    assert!(
        hei.chars.contains(&'中') && hei.chars.contains(&'第'),
        "\\r 之后回到 style 的 SimHei：{hei:?}"
    );
    // 第一行 "粗体{\b1}加粗{\b0}收尾"：style 基准就是粗体，{\b0} 之后才分裂出常规那条
    let normal = got
        .iter()
        .find(|r| r.family == "Arial" && !r.bold)
        .expect("{\\b0} 之后要另记一条常规");
    assert!(
        normal.chars.contains(&'收') && !normal.chars.contains(&'粗'),
        "style 基准是粗体，只有 \\b0 之后的字才属常规：{normal:?}"
    );
    let bold = got
        .iter()
        .find(|r| r.family == "Arial" && r.bold)
        .expect("style 基准（粗体）那条必须存在");
    assert!(
        bold.chars.contains(&'粗') && bold.chars.contains(&'加'),
        "开头两字与 {{\\b1}} 期间的字都属粗体：{bold:?}"
    );
}

/// drawing 的命令串、`\N` 这类转义都不是"需要覆盖的字符"（17 §3 的字集要干净）。
#[test]
fn drawing_and_escapes_are_not_characters() {
    let src = "[V4+ Styles]\r\nFormat: Name, Fontname\r\nStyle: D,Arial\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,D,{\\p1}m 0 0 l 10 10{\\p0}\\N好\r\n";
    let got = scan_text(src).unwrap();
    let a = req(&got, "Arial").unwrap();
    assert!(a.chars.contains(&'好'), "真字符要在：{a:?}");
    for c in ['m', 'l', 'N', '1', '0', '\\'] {
        assert!(
            !a.chars.contains(&c),
            "drawing 命令或转义被当成字符了：{c:?} in {:?}",
            a.chars
        );
    }
    assert_eq!(a.pen, 1, "只该有一次落笔：{a:?}");
}

/// 报告要能 diff、要能重放（20 §Repro）：同一份文档两次扫描完全相同，且按 face 状态排序。
#[test]
fn scanning_is_deterministic_and_ordered() {
    let a = scan_text(ASS).unwrap();
    let b = scan_text(ASS).unwrap();
    assert_eq!(a, b);
    let fams: Vec<String> = a.iter().map(|r| r.family.clone()).collect();
    let mut sorted = fams.clone();
    sorted.sort();
    assert_eq!(fams, sorted, "{fams:?}");
    assert!(
        a.iter().all(|r| !r.sources.is_empty()),
        "每条用量都要能说清是谁引入的：{a:?}"
    );
}

/// 同一家族的不同字重/斜体必须是两条记录 —— 否则"SimHei 常规"与"SimHei 斜体"
/// 会被合成一条，缺字报告也就跟着错了。
#[test]
fn weight_and_slant_are_separate_requests() {
    let src = "[V4+ Styles]\r\nFormat: Name, Fontname, Bold, Italic\r\nStyle: A,Arial,0,0\r\nStyle: B,Arial,-1,0\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,A,甲\r\nDialogue: 0,0:00:03.00,0:00:04.00,B,乙\r\n";
    let got = scan_text(src).unwrap();
    let arial: Vec<&FaceRequest> = got.iter().filter(|r| r.family == "Arial").collect();
    assert_eq!(arial.len(), 2, "常规与粗体该分开记账：{got:?}");
    assert!(arial.iter().any(|r| r.bold && r.chars.contains(&'乙')));
    assert!(arial.iter().any(|r| !r.bold && r.chars.contains(&'甲')));
}

/// 空文档/无 Events 必须给可报告的错误，而不是给一份"看起来没问题"的空报告。
#[test]
fn missing_events_section_is_an_error_not_an_empty_report() {
    let err = scan_text("[Script Info]\r\nPlayResX: 1920\r\n").unwrap_err();
    assert_eq!(err.code_string(), "STK-INP-0230", "{err:?}");
    assert!(err.title.contains("Events"), "{err}");
}
