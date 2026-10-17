//! 用**本机真实字体库**验证 17 §2/§4 与 AT-FONT-001/002。
//!
//! 为什么不能只用合成 fixture：一个最小的合法 TTF 里我可以随意摆 name 表与 cmap，
//! 那样"我们读对了 TrueType"这句话就没有被检验过。这里读的是系统目录里真实的
//! TTF/TTC，包括中文本地化名与集合成员索引。
//!
//! 需要外部资源，所以与 libass/ffprobe 同一套规矩：默认 SKIP，`STK_REQUIRE_FONTS=1`
//! （门阶用）时"没跑"直接算失败。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use font_service::db::{FontDb, NameMatch};
use font_service::report::{evaluate, Verdict};
use font_service::usage::FaceRequest;

fn fonts_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_FONTS_DIR") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return Some(p);
        }
        return None;
    }
    if std::env::var("STK_REQUIRE_FONTS").is_err() {
        return None;
    }
    let windir = std::env::var("WINDIR").ok()?;
    let p = Path::new(&windir).join("Fonts");
    p.is_dir().then_some(p)
}

macro_rules! need_fonts {
    () => {
        match fonts_dir() {
            Some(p) => p,
            None => {
                if std::env::var("STK_REQUIRE_FONTS").is_ok() {
                    panic!("STK_REQUIRE_FONTS=1，但找不到字体目录（设 STK_FONTS_DIR）");
                }
                eprintln!("SKIP：未启用真实系统字体扫描测试（设 STK_REQUIRE_FONTS=1 开启）");
                return;
            }
        }
    };
}

fn request(family: &str, bold: bool, italic: bool, chars: &str) -> FaceRequest {
    FaceRequest {
        family: family.to_string(),
        bold,
        italic,
        chars: chars.chars().collect(),
        pen: chars.chars().count(),
        sources: vec!["test".to_string()],
    }
}

/// 17 §2：数据库要记的不只是文件名。这里逐项验"真的从表里读出来了"。
#[test]
fn a_real_font_library_is_read_from_its_tables_not_its_filenames() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    assert!(
        db.faces.len() > 30,
        "{} 下至少该读出几十条 face：{}",
        dir.display(),
        db.faces.len()
    );
    assert!(db.files_read > 10, "文件数也要报：{}", db.files_read);
    for f in &db.faces {
        assert!(!f.family.is_empty(), "{f:?} 没有家族名");
        assert_eq!(f.sha256.len(), 64, "身份必须是 sha256：{f:?}");
        assert!(f.num_glyphs > 0, "{f:?} 的 cmap 是空的？");
        assert!(f.units_per_em > 0, "{f:?} 没有 head 表？");
        assert!((1..=1000).contains(&f.weight), "{f:?} 的字重超出 OS/2 范围");
    }
    // 每一条"没读成"都必须有原因，静默少扫等于报告在撒谎
    for n in &db.notes {
        assert!(n.reason.chars().count() > 4, "{n:?}");
    }

    // TTC/OTC：同一个文件里多个 face，且各自的 name 表不同
    let collections: Vec<&_> = db.faces.iter().filter(|f| f.collection_len > 1).collect();
    assert!(
        !collections.is_empty(),
        "本机字体库里该有 TrueType 集合（msyh.ttc / Sitka.ttc 之类）"
    );
    let one = collections[0];
    let siblings: Vec<&_> = db
        .faces
        .iter()
        .filter(|f| f.file == one.file && f.face_index != one.face_index)
        .collect();
    assert!(!siblings.is_empty(), "集合成员索引没读对：{one:?}");
    assert_ne!(
        one.face_index, siblings[0].face_index,
        "同一文件内两个 face 必须有不同索引"
    );

    // 本地化家族名：中文 Windows 上"微软雅黑"这类名字必须被记进别名，
    // 否则 §2 说的"localized family names"就是没做。
    let localized = db
        .faces
        .iter()
        .filter(|f| f.family_aliases.iter().any(|a| !a.is_ascii()))
        .count();
    assert!(
        localized > 0,
        "没有任何 face 带非 ASCII 的本地化家族名：扫描是否只读了 platform 3？"
    );
}

/// AT-FONT-001：中文字体缺字要被报告 —— 两个方向都要验。
#[test]
fn at_font_001_missing_cjk_glyphs_are_reported_and_a_covering_font_is_clean() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    let probe = "你好";

    // 方向一：找一个真的覆盖"你"的 face，它不该被报缺字
    let mut covering: Option<usize> = None;
    for (idx, _rec) in db.faces.iter().enumerate() {
        let chars: BTreeSet<char> = ['你'].into_iter().collect();
        match db.missing_glyphs(idx, &chars) {
            Ok(missing) => {
                if missing.is_empty() {
                    covering = Some(idx);
                    break;
                }
            }
            // 集合成员/损坏字体读不出来是可报告的错误，不该让整轮扫描失败
            Err(e) => {
                assert!(e.title.len() > 4, "{e:?}");
            }
        }
    }
    let idx =
        covering.expect("本机字体库里该有任何一个 face 能画\"你\"（CJK 字体缺失则这条测不了）");
    let chars: BTreeSet<char> = probe.chars().collect();
    assert!(
        db.missing_glyphs(idx, &chars).unwrap().is_empty(),
        "{} 能画\"你\"，就不该被报缺字",
        db.faces[idx].label()
    );

    // 方向二：一个只覆盖拉丁的 face（拿库里最"西方"的那条）必须被报缺字
    let latin_only = db
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.family_aliases.is_empty() || f.family_aliases.iter().all(|a| a.is_ascii())
        })
        .map(|(i, _)| i)
        .find(|i| {
            db.missing_glyphs(*i, &chars)
                .map(|m| !m.is_empty())
                .unwrap_or(false)
        });
    let idx2 = latin_only.expect("拉丁 only 的 face 必须被报出中文字符缺失");
    let missing: String = db
        .missing_glyphs(idx2, &chars)
        .unwrap()
        .into_iter()
        .collect();
    assert!(
        !missing.is_ascii(),
        "{} 缺字清单里没有中文字符：{missing}",
        db.faces[idx2].label()
    );

    // 评级层也要能把这件事说清楚：报告里 missing_glyphs 非空
    let m = &evaluate(&db, &[request(&db.faces[idx2].family, false, false, probe)]).unwrap()[0];
    assert!(!m.missing_glyphs.is_empty(), "{m:?}");
    assert!(
        !m.missing_glyphs.is_ascii(),
        "评级报告里的缺字必须包含那批中文字符：{m:?}"
    );
}

/// 17 §4 的第 5 档：名字明明对上了，但那个 face 画不出正文 —— 必须改判 `MissingGlyph`，
/// 而不是挂着 `Exact` 再在旁边的字段里塞一份缺字清单。
///
/// 这条为什么值得单独测：把"能不能对上名字"与"能不能画出字"合进一个结论，
/// 是报告最容易骗人的地方（脚本只读 verdict 就会以为万事大吉）。
#[test]
fn a_named_face_that_cannot_draw_the_text_is_relabelled_missing_glyph() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    let probe: BTreeSet<char> = ['你'].into_iter().collect();

    // 稳定条件：同名的每一条 face 都缺这个字。否则"改判"取决于库里哪条先被扫到，
    // 测的就不是评级而是扫描次序。
    let mut family: Option<String> = None;
    for (i, f) in db.faces.iter().enumerate() {
        if db
            .missing_glyphs(i, &probe)
            .map(|m| m.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        let same: Vec<usize> = db
            .faces
            .iter()
            .enumerate()
            .filter(|(_, g)| g.family == f.family)
            .map(|(j, _)| j)
            .collect();
        if same.iter().all(|j| {
            db.missing_glyphs(*j, &probe)
                .map(|m| !m.is_empty())
                .unwrap_or(false)
        }) && !same.is_empty()
        {
            family = Some(f.family.clone());
            break;
        }
    }
    let want = family.expect("本机字体库里该有一个画不出\"你\"的拉丁字体家族");
    let m = &evaluate(&db, &[request(&want, false, false, "你")]).unwrap()[0];
    assert_eq!(m.verdict, Verdict::MissingGlyph, "{m:?}");
    assert_eq!(m.missing_glyphs, "你", "{m:?}");
    // 改判不能丢掉指路：仍然要告诉用户"是哪条 face 名字对上了但覆盖不够"
    assert!(m.face.is_some(), "{m:?}");
    assert_eq!(m.face.as_ref().unwrap().family, want, "{m:?}");
    assert!(m.why.contains("缺"), "why 必须说清缺什么：{m:?}");
}

/// 17 §4 的评级要能区分"名字对上"与"只能替代"与"根本没有"。
#[test]
fn verdicts_separate_exact_alias_substituted_and_missing() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    let some_family = db
        .faces
        .iter()
        .find(|f| f.family.is_ascii() && f.family.len() > 3)
        .expect("库里该有一个 ASCII 家族名")
        .family
        .clone();

    let exact = &evaluate(&db, &[request(&some_family, false, false, "abc")]).unwrap()[0];
    assert!(
        matches!(exact.verdict, Verdict::Exact | Verdict::Alias),
        "{some_family} 明明在库里：{exact:?}"
    );
    assert!(exact.face.is_some(), "{exact:?}");
    // 候选必须能反查到具体文件（集合成员要带索引），否则报告等于没指路
    for c in &exact.candidates {
        assert!(
            c.file.contains('.') && !c.family.is_empty(),
            "候选指不到文件：{exact:?}"
        );
    }

    // 完全不同的名字：只能是 Missing（或按共同词 Substitute）
    let none = &evaluate(
        &db,
        &[request(
            "Zzzx-Definitely-Not-Installed-Family",
            false,
            false,
            "abc",
        )],
    )
    .unwrap()[0];
    assert!(
        matches!(none.verdict, Verdict::Missing | Verdict::Substituted),
        "不存在的家族被判成 {none:?}"
    );
    if none.verdict == Verdict::Missing {
        assert!(none.face.is_none(), "{none:?}");
        assert!(none.why.contains("找不到"), "{none:?}");
    }

    // 有共同词但整名不存在 → Substituted，而且必须明说"这是替代不是等价"
    let near = &evaluate(
        &db,
        &[request(
            &format!("{some_family} UltraExpanded"),
            false,
            false,
            "abc",
        )],
    )
    .unwrap()[0];
    if let Some(face) = &near.face {
        assert_ne!(
            face.family.to_lowercase(),
            near.request.to_lowercase(),
            "落到的 face 与请求整名相同就不该走替代分支：{near:?}"
        );
        if near.verdict == Verdict::Substituted {
            assert!(near.why.contains("替代"), "{near:?}");
            assert!(!near.candidates.is_empty(), "替代要给出候选：{near:?}");
        }
    }
    assert!(
        !matches!(near.verdict, Verdict::Exact),
        "加了后缀的家族名不该仍然精确命中同名 face：{near:?}"
    );
}

/// AT-FONT-002：字体环境变了，基于旧库的结论必须失效，而不是继续给一个过期答案。
#[test]
fn at_font_002_changed_font_content_invalidates_the_stale_database() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    let rec = db
        .faces
        .iter()
        .enumerate()
        .find(|(_, f)| f.file.extension().and_then(|s| s.to_str()) == Some("ttf"))
        .map(|(i, f)| (i, f.clone()))
        .expect("库里该有一个 .ttf");

    let tmp = std::env::temp_dir().join(format!("stk-font-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let copy = tmp.join(rec.1.file.file_name().unwrap());
    std::fs::copy(&rec.1.file, &copy).unwrap();

    let mut db2 = FontDb::scan(&[tmp.as_path()]);
    assert_eq!(
        db2.faces.len(),
        1,
        "临时目录里只有一个文件：{:?}",
        db2.faces
    );
    let chars: BTreeSet<char> = ['a'].into_iter().collect();
    assert!(db2.missing_glyphs(0, &chars).is_ok(), "刚扫完就该能判定");

    // 用户在扫描之后换掉了字体文件（升级、注销、覆盖安装都会这样）
    let mut bytes = std::fs::read(&copy).unwrap();
    let len = bytes.len();
    bytes[len - 1] ^= 0xff;
    std::fs::write(&copy, &bytes).unwrap();
    let err = db2
        .missing_glyphs(0, &chars)
        .expect_err("字体内容变了就必须拒绝给结论");
    assert_eq!(err.code_string(), "STK-INP-0222", "{err:?}");
    assert!(err.title.contains("变了"), "{err}");

    // 重扫之后结论恢复可用：这就是"cache key 含字体哈希"的那半边
    db2 = FontDb::scan(&[tmp.as_path()]);
    assert_ne!(
        db2.faces[0].sha256, rec.1.sha256,
        "哈希必须能看出文件被改过"
    );
    let again = db2.missing_glyphs(0, &chars);
    // 改掉最后一个字节可能让整个文件不可解析——那也是"被发现了"，不是静默通过
    assert!(
        again.is_ok() || again.unwrap_err().code_string().starts_with("STK-INP-022"),
        "要么给出结论，要么给出可报告的错误"
    );
    std::fs::remove_dir_all(&tmp).ok();
}

/// 匹配入口本身要能区分"哪个名字对上"（17 §4 的第 2 级：别名）。
#[test]
fn postscript_and_localized_names_are_matchable_not_just_family() {
    let dir = need_fonts!();
    let db = FontDb::scan(&[dir.as_path()]);
    let by_ps = db.faces.iter().enumerate().find(|(_, f)| {
        !f.postscript_name.is_empty() && f.matches_name(&f.postscript_name) == NameMatch::PostScript
    });
    assert!(
        by_ps.is_some(),
        "PostScript 名必须能反查到自己（17 §2 记这个字段就是为了能用）"
    );
    let localized = db.faces.iter().find(|f| {
        f.family_aliases
            .iter()
            .any(|a| !a.is_ascii() && f.matches_name(a) == NameMatch::Localized)
    });
    assert!(localized.is_some(), "本地化家族名必须能反查到自己");
}

/// 注册表带来的文件必须真的进库，且与目录扫到的同一文件不重复计。
///
/// 本机所有登记项都指向 `%WINDIR%\Fonts` 里的文件，所以"注册在别处的字体"这一情况
/// 在这台机器上无法用真注册表演示；这里用同一个入口函数把它演出来：
/// 一个空目录 + 一个不在目录里的文件。入口一样，语义就一样。
#[test]
fn registry_supplied_files_enter_the_database_without_double_counting() {
    let dir = need_fonts!();
    let db_dirs = FontDb::scan(&[dir.as_path()]);
    assert!(!db_dirs.faces.is_empty(), "本机字体库不该是空的");

    let tmp = std::env::temp_dir().join(format!("stk-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let empty = tmp.join("empty");
    let elsewhere = tmp.join("elsewhere");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let src = db_dirs
        .faces
        .iter()
        .find(|f| f.file.extension().and_then(|s| s.to_str()) == Some("ttf"))
        .map(|f| f.file.clone())
        .expect("库里该有一个 .ttf");
    let copy = elsewhere.join(src.file_name().unwrap());
    std::fs::copy(&src, &copy).unwrap();

    // 只有空目录：没有注册表就一个 face 都没有
    let alone = FontDb::scan(&[empty.as_path()]);
    assert!(alone.faces.is_empty(), "{alone:?}");
    // 同一个路径既被目录扫到、又被注册表指到：只能算一次，否则 files_read 与 face 都在虚报
    let overlap = FontDb::scan_registered(&[elsewhere.as_path()], std::slice::from_ref(&copy));
    assert_eq!(
        overlap.files_read, 1,
        "同一文件被两个来源指到不能算两次：{:?}",
        overlap.faces
    );
    assert_eq!(overlap.faces.len(), 1, "{:?}", overlap.faces);
    assert_eq!(
        overlap.registry_only, 0,
        "目录里已经有了，不该再记成注册表带来的"
    );
    // 目录与注册表各给一份不重叠的东西：两份都进库，注册表那一份被计数
    let disjoint = FontDb::scan_registered(&[elsewhere.as_path()], std::slice::from_ref(&src));
    assert_eq!(disjoint.files_read, 2, "{:?}", disjoint.files_read);
    assert_eq!(disjoint.registry_only, 1, "{:?}", disjoint.registry_only);
    std::fs::remove_dir_all(&tmp).ok();
}
