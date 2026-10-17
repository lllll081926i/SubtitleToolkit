//! 17 §2 的权威交叉核对：**真机上的** GDI 枚举行为。
//!
//! 单元测试（`font-service/src/gdi.rs`）用的是构造出来的 `GdiFace`，
//! 验的是对照逻辑；这一组验的是**真的调 GDI 能不能拿到东西、拿回来的东西像不像字体**。
//!
//! 两者都必须有：只测对照逻辑的话，`EnumFontFamiliesExW` 的声明写错
//! （结构体字段宽度/顺序、回调签名、`charset` 取值）会一路绿过去，
//! 而那类错误的表现是"枚举到 0 个"或"读到垃圾名字"。

use font_service::gdi;

/// 真机枚举必须拿到相当数量的**真曲线字体**。
///
/// 本机实测原始 6020 个 face，其中大部分是 System/Terminal/Fixedsys 这类
/// 不带 `name`/`cmap` 表的古早位图字体，过滤后才是可与我们的库比较的范围。
///
/// 这里用 100 当下界：任何一台正常装了 Windows 的机器都远超它（中文字体
/// 一套就好几个），而结构体声明写错时会掉到 0。
#[test]
fn the_real_enumeration_returns_truetype_fonts() {
    match gdi::enumerate_for_test() {
        Ok(faces) => {
            assert!(
                !faces.is_empty(),
                "真机上该枚举到系统字体（0 个说明 GDI 绑定写错了）"
            );
            let curve: Vec<_> = faces
                .iter()
                .filter(|f| f.truetype && !f.family.starts_with('@'))
                .collect();
            assert!(
                curve.len() > 100,
                "该有 100 个以上真曲线字体，实际 {}（原始 {} 个 face）",
                curve.len(),
                faces.len()
            );
            // 名字必须读得出来，而且是像样的字（不是乱码）。
            // 结构体偏移写错时读出来的是别的字段的字节。
            let named = curve.iter().filter(|f| !f.family.is_empty()).count();
            assert_eq!(named, curve.len(), "每个 face 都该有家族名");
            assert!(
                curve
                    .iter()
                    .any(|f| f.family.chars().any(|c| c.is_ascii_alphabetic())),
                "至少有一个家族名含 ASCII 字母（读成乱码时会失败）"
            );
        }
        Err(why) => {
            // 无桌面会话（服务/容器）时 GDI 可能拿不到 DC。那是环境事实，不是代码错。
            eprintln!("SKIP：本机 GDI 不可用（{why}）");
        }
    }
}

/// 枚举必须包含几套**人人都有的**字体。
///
/// 这条比"数量够多"更强：数量够多但偏移错位也能凑出 100 个垃圾条目，
/// 而 `Arial` 这种名字只能从正确的位置读出来。
#[test]
fn the_enumeration_contains_well_known_system_families() {
    let Ok(faces) = gdi::enumerate_for_test() else {
        eprintln!("SKIP：本机 GDI 不可用");
        return;
    };
    let names: std::collections::BTreeSet<String> = faces
        .iter()
        .map(|f| f.family.to_lowercase())
        .filter(|n| !n.is_empty())
        .collect();
    // Windows 上这套一定在（TrueType 版）
    assert!(
        names.contains("arial"),
        "该枚举到 Arial。实际前 20 个：{:?}",
        names.iter().take(20).collect::<Vec<_>>()
    );
    assert!(
        names.contains("times new roman"),
        "该枚举到 Times New Roman"
    );
}

/// 真枚举与真字体库对照：**两台机器上结论不同是正常的**，
/// 所以这里只断言"核对跑过、且给出的是可读结论"，不断言缺口数量。
///
/// 具体缺什么取决于本机装了什么字体 —— 那正是这个功能要回答的问题，
/// 而不是测试能预先知道的答案。
#[test]
fn a_real_cross_check_produces_readable_evidence() {
    let Ok(faces) = gdi::enumerate_for_test() else {
        eprintln!("SKIP：本机 GDI 不可用");
        return;
    };
    // 只拿 GDI 的家族名当"我们的库"（等价于"库里有 GDI 看到的全部"），
    // 于是 missing 必然为空 —— 这是负基线：核对器不该凭空造出缺口。
    let all_names: Vec<String> = faces.iter().map(|f| f.family.clone()).collect();
    let c = gdi::cross_check_for_test(&all_names, faces);
    assert!(c.ran, "该跑过：{c:?}");
    assert_eq!(c.db_faces, 0, "本用例没给 db_faces");
    assert!(
        c.missing_from_db.is_empty(),
        "库里有全部 GDI 名字时不该报缺口：{:?}",
        c.missing_from_db
    );
    assert!(!c.has_gap());
    assert!(c.gdi_faces_raw > 0, "原始计数该 > 0：{c:?}");
}
