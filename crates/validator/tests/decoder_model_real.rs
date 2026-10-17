//! 第 5 层（解码器模型）对着**真实第三方商业 SUP** 的回归。
//!
//! ## 为什么必须用真语料
//!
//! 单测里的夹具是我按真实流的骨架手写的。手写夹具只能证明"实现符合我的理解"，
//! 证明不了"我的理解符合真实世界"。这一层尤其危险：如果不变式选错了，
//! 它会对所有真实文件刷一堆假阳性——而假阳性会让用户干脆不用这一层。
//!
//! 本机实测（真实 98.97 MB SUP，9951 个显示集 / 49659 段）：
//! 5 条不变式**全部成立**，0 条违例。这就是这一层选对了的直接证据。
//!
//! ## 语料从哪来
//!
//! 用 `STK_REAL_SUP` 指定路径。没有就**明确跳过**（打印理由），不算通过——
//! 把"没跑"写成"通过"是这一路最要避免的事。
//!
//! 真实商业语料不可入库（24 §语料授权），所以这里只能按环境变量可选执行。

use codec_pgs::segment::parse_stream;
use validator::decoder_model::{decoder_model, summary};
use validator::report::Severity;

/// 找一份真实 SUP：`STK_REAL_SUP` 优先，其次看本机约定位置。
fn real_sup() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("STK_REAL_SUP") {
        let pb = std::path::PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    // 不要把某个具体片名写死在这里——那是把本机路径当成仓库约定。
    // 只认环境变量；`STK_REQUIRE_REAL_SUP` 用来在门阶上要求它必须存在。
    None
}

fn load() -> Option<Vec<codec_pgs::segment::Segment>> {
    let path = real_sup()?;
    let bytes = std::fs::read(&path).expect("真实 SUP 必须可读");
    Some(parse_stream(&bytes).expect("真实 SUP 必须能解析"))
}

#[test]
fn the_five_invariants_hold_on_the_real_commercial_corpus() {
    let Some(segments) = load() else {
        if std::env::var("STK_REQUIRE_REAL_SUP").is_ok() {
            panic!("设了 STK_REQUIRE_REAL_SUP 但没有可用的 STK_REAL_SUP，不能静默跳过");
        }
        eprintln!(
            "跳过：未提供 STK_REAL_SUP（真实商业语料不可入库，见 24 §语料授权）。\
             设 STK_REAL_SUP=<path>.sup 可执行本用例。"
        );
        return;
    };

    let s = summary(&segments);
    println!(
        "真实语料：{} 段 / {} 个显示集；\
         违反 PTS<=PCS {} 次、DTS<=PTS {} 次、DTS 递减 {} 次",
        s.segments, s.sets, s.pts_le_pcs_violations, s.dts_gt_pts_violations, s.dts_drops
    );

    // 语料规模要够大，否则"全过"没有说服力
    assert!(
        s.segments > 1000 && s.sets > 100,
        "语料太小，不足以支撑结论：{s:?}"
    );

    // 核心判据：5 条不变式在真实商业流上必须 0 违例
    assert_eq!(s.pts_le_pcs_violations, 0, "真实流违反了 PTS<=PCS：{s:?}");
    assert_eq!(s.dts_gt_pts_violations, 0, "真实流违反了 DTS<=PTS：{s:?}");
    assert_eq!(s.dts_drops, 0, "真实流违反了 DTS 单调：{s:?}");

    // 报告层：不该出现 warning 以上的层 5 发现
    let (r, checked) = decoder_model(&segments);
    assert!(checked, "有段就必须声称跑过");
    let bad: Vec<_> = r
        .findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning && f.check.starts_with("PGS.DEC."))
        .map(|f| (f.check, f.message.as_str()))
        .collect();
    assert!(
        bad.is_empty(),
        "真实商业流不该被报出层 5 问题（那说明不变式选错了）：{bad:?}"
    );
    // 必须有一条"全部成立"，否则报告看不出这层跑过
    assert!(
        r.findings.iter().any(|f| f.check == "PGS.DEC.10"),
        "{:?}",
        r.findings
    );
}

/// 反向对照：真实流被**故意改坏**之后，本层必须能抓到。
///
/// 只证"真实流不报错"是不够的——一个什么都不检查的实现同样不报错。
/// 这里把真实流的 DTS 拧乱，确认门禁会响。
#[test]
fn the_same_real_corpus_is_caught_once_it_is_actually_broken() {
    let Some(mut segments) = load() else {
        eprintln!("跳过：未提供 STK_REAL_SUP");
        return;
    };

    // 先确认原样是干净的
    let (clean, _) = decoder_model(&segments);
    let clean_bad = clean
        .findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning && f.check.starts_with("PGS.DEC."))
        .count();
    assert_eq!(clean_bad, 0, "对照组必须先干净");

    // 找一个靠后的段，把它的 PTS 抬到远超本集 PCS
    let victim = segments.len() / 2;
    segments[victim].pts = segments[victim].pts.saturating_add(900_000);

    let (dirty, _) = decoder_model(&segments);
    let hits: Vec<&str> = dirty
        .findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning && f.check.starts_with("PGS.DEC."))
        .map(|f| f.check)
        .collect();
    assert!(
        !hits.is_empty(),
        "把真实流改坏之后本层必须报出来，否则它只是个摆设"
    );
}
