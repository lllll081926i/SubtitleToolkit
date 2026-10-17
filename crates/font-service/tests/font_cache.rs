//! 字体库磁盘缓存的验收（17 §2 的"数据库"、AI5 的性能债）。
//!
//! 这里要守住的不是"快"，而是**缓存只能让我们慢，不能让我们答错**：
//! * 命中与全量扫描必须给出同一份库、同一个渲染身份；
//! * 指纹（大小/mtime）被欺骗时，使用点的内容哈希复核必须把它拦下来并退回全量重扫；
//! * 缓存坏了/版本不符就退回全量扫描，而不是让命令失败；
//! * `--no-font-cache` 那条路必须真的不碰缓存文件。
//!
//! 用临时目录里复制来的一份真字体跑，不需要 env 门控：被测的是缓存逻辑，
//! 不是"这台机器装了什么字体"。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use font_service::cache::{self, CACHE_VERSION};
use font_service::db::{CacheMode, CacheState, FontDb, FONT_EXTS};
use font_service::usage::FaceRequest;
use font_service::{evaluate, evaluate_or_rescan, identify};

/// 缓存路径靠环境变量决定，而它是进程级的：同一二进制里的测试必须串行改它。
fn env_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-fcache-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 找一份真字体复制进 `dir`。缓存要核的是内容哈希，合成字节做不到这件事。
fn one_font(dir: &Path) -> PathBuf {
    let windir = std::env::var("WINDIR").expect("测试需要 WINDIR");
    let sys = Path::new(&windir).join("Fonts");
    let mut picks: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&sys) {
        for e in rd.flatten() {
            let p = e.path();
            let hit = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| FONT_EXTS.contains(&s.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if hit && p.is_file() {
                picks.push(p);
            }
        }
    }
    picks.sort();
    let src = picks
        .into_iter()
        .next()
        .expect("本机字体目录里该有可读的字体文件");
    let to = dir.join(src.file_name().unwrap());
    std::fs::copy(&src, &to).unwrap();
    to
}

/// 另一个可解析的字体文件（与 `one_font` 不同的一份），用来制造"内容换了但还是字体"。
fn second_font(not_this: &Path) -> PathBuf {
    let windir = std::env::var("WINDIR").unwrap();
    let sys = Path::new(&windir).join("Fonts");
    let mut picks: Vec<PathBuf> = std::fs::read_dir(&sys)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| FONT_EXTS.contains(&s.to_ascii_lowercase().as_str()))
                    .unwrap_or(false)
        })
        .collect();
    picks.sort();
    picks
        .into_iter()
        .find(|p| p != not_this && p.file_name() != not_this.file_name())
        .expect("本机至少有两个可读字体文件")
}

/// 请求 `db` 里第一条 face 的家族名 —— 一定要用真的那一个名字，否则匹配不到 face，
/// 使用点复核根本没机会跑起来（`missing_glyphs` 只在真的选中某条 face 时才读文件）。
fn request_for(db: &FontDb) -> Vec<FaceRequest> {
    let f = db.faces.first().expect("这份库该有 face");
    vec![FaceRequest {
        family: f.family.clone(),
        bold: false,
        italic: false,
        chars: ['A', '你'].into_iter().collect(),
        pen: 1,
        sources: vec!["test".to_string()],
    }]
}

/// 命中与全量必须给同一份库；而且第二次真的走了缓存。
#[test]
fn a_hit_returns_the_same_database_as_a_full_scan() {
    let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("hit");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    one_font(&fonts);
    std::env::set_var("STK_FONTCACHE", dir.join("cache.json"));

    let first = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert_eq!(
        first.cache,
        CacheState::Scanned,
        "第一次必须是全量：{first:?}"
    );
    assert!(dir.join("cache.json").exists(), "扫完就该留下缓存文件");
    let second = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert_eq!(second.cache, CacheState::Hit, "第二次应该命中缓存");
    assert_eq!(
        second.faces, first.faces,
        "命中缓存给出的库必须与全量扫描逐条相同"
    );
    assert_eq!(second.files_read, first.files_read);
    let plain = FontDb::scan(&[fonts.as_path()]);
    assert_eq!(plain.faces, first.faces, "缓存路径不能改变答案本身");
    std::env::remove_var("STK_FONTCACHE");
    std::fs::remove_dir_all(&dir).ok();
}

/// 文件被改过（大小+mtime 都变了）→ 指纹失效 → 重新扫描并给出新哈希。
#[test]
fn a_changed_font_file_invalidates_the_fingerprint() {
    let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("fingerprint");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let f = one_font(&fonts);
    std::env::set_var("STK_FONTCACHE", dir.join("cache.json"));

    let before = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert!(
        !before.faces.is_empty(),
        "第一次扫描就该有 face：{before:?}"
    );
    // 换一个**仍然可解析**的内容：把另一个真字体盖上去。
    // （第一版这里用 `bytes.reverse()` 制造"内容变了"，结果文件根本解析不了，
    //  于是 0 条 face 是产品的正确回答 —— 断言的语义又写歪了，同族第 5 次。）
    let other = second_font(&f);
    std::fs::copy(&other, &f).unwrap();
    let after = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert!(!after.faces.is_empty(), "换一个真字体后不该空：{after:?}");
    assert_eq!(after.cache, CacheState::Scanned, "文件变了必须重扫");
    assert_ne!(
        before.faces[0].sha256, after.faces[0].sha256,
        "重扫之后记录的内容哈希必须换成新的：{} vs {}",
        before.faces[0].sha256, after.faces[0].sha256
    );
    std::env::remove_var("STK_FONTCACHE");
    std::fs::remove_dir_all(&dir).ok();
}

/// 最坏情况：内容变了但大小与 mtime 都没变（指纹看不出问题），缓存里还写着旧哈希。
/// 这条测试锁的是整份设计的立足点：**发现它的地方是使用点，而不是命中前的全库复核**
/// （复核整座库的代价 ≈ 重扫一遍，那样缓存就没意义了，见 ISSUES AI5 的实测）。
#[test]
fn a_stale_index_is_caught_where_the_bytes_are_used() {
    let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("stale-index");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    one_font(&fonts);
    let cpath = dir.join("cache.json");
    std::env::set_var("STK_FONTCACHE", &cpath);

    let good = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    // 手工写一份"指纹正确但哈希是假的"缓存：模拟指纹骗人的一种极端形式
    let mut stale = good.clone();
    for rec in stale.faces.iter_mut() {
        rec.sha256 = "0".repeat(64);
    }
    let files: Vec<PathBuf> = std::fs::read_dir(&fonts)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    let mut fp = cache::fingerprint(&files);
    fp.push("registry=0".to_string());
    cache::store(&fp, &stale);
    assert!(cpath.exists());

    // 缓存这层照旧命中：它没有义务读每个文件（那样就等于没有缓存）
    let hit = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert_eq!(hit.cache, CacheState::Hit, "假哈希这层看不出来，是设计如此");
    let requests = request_for(&hit);
    // 但**用**这份索引去判覆盖就必须炸：读文件、比哈希
    let boom = evaluate(&hit, &requests);
    assert!(
        boom.is_err(),
        "内容哈希是假的还判得出覆盖，那使用点复核就是装饰"
    );

    // 走真正的入口：它会把这次退回全量重扫，并给出与"压根没用缓存"一致的答案
    let mut db = hit;
    let m = evaluate_or_rescan(&mut db, &[fonts.as_path()], &[], &requests).expect("该重扫一次");
    assert_eq!(db.cache, CacheState::Refreshed, "{:?}", db.cache);
    assert!(db.faces.iter().all(|r| r.sha256 != "0".repeat(64)));
    let plain = FontDb::scan(&[fonts.as_path()]);
    let expect = evaluate(&plain, &requests).expect("全量扫描该能判");
    assert_eq!(
        m.iter().map(|x| x.verdict).collect::<Vec<_>>(),
        expect.iter().map(|x| x.verdict).collect::<Vec<_>>(),
        "重扫后的结论必须与全量扫描一致"
    );
    std::env::remove_var("STK_FONTCACHE");
    std::fs::remove_dir_all(&dir).ok();
}

/// 坏缓存（JSON 读不懂 / 版本不符 / 空 face）一律“用坏即弃”：
/// 退回全量扫描（不能把用户挡在门外），并且 `load` 必须把那份缓存文件删掉，
/// 而不是留着等下次再碰一遍同一块石头。
#[test]
fn a_broken_cache_file_is_deleted_rather_than_kept() {
    let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("broken");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    one_font(&fonts);
    let cpath = dir.join("cache.json");
    std::env::set_var("STK_FONTCACHE", &cpath);

    std::fs::write(&cpath, b"not json at all").unwrap();
    let db = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert_eq!(db.cache, CacheState::Scanned, "读不懂就当没命中：{db:?}");
    assert!(!db.faces.is_empty(), "退回全量扫描后必须仍有答案");

    // 版本换了也要作废：记录结构变了还吃旧缓存 = 读到错的东西。
    // 先建一份好缓存，再把版本字段改成不兼容的值。
    let good = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    let raw = std::fs::read_to_string(&cpath).unwrap();
    std::fs::write(&cpath, raw.replace(CACHE_VERSION, "stk-fontdb-v0-不再兼容")).unwrap();
    assert!(cpath.exists(), "前提：此时磁盘上确实有一份版本不符的缓存");
    let after = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    assert_eq!(after.cache, CacheState::Scanned, "版本不符的缓存不能吃");
    assert_eq!(after.faces, good.faces);

    // 最后直接验证“坏就用坏即弃”的契约：不可解析字节会被删掉
    std::fs::write(&cpath, b"not json at all").unwrap();
    let files: Vec<PathBuf> = std::fs::read_dir(&fonts)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    let mut fp = cache::fingerprint(&files);
    fp.push("registry=0".to_string());
    assert!(cache::load(&fp).is_none(), "读不懂的缓存不能当命中");
    assert!(!cpath.exists(), "坏缓存必须被删掉，而不是留在磁盘上");

    std::env::remove_var("STK_FONTCACHE");
    std::fs::remove_dir_all(&dir).ok();
}

/// 重扫后身份不变：缓存命中、退回重扫、完全不用缓存，三条路必须给出同一个摘要。
/// 否则"有没有缓存"本身就会让渲染缓存反复失效（`CacheState` 是成本事实，不进摘要）。
#[test]
fn every_cache_path_gives_the_same_render_identity() {
    let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("identity");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    one_font(&fonts);
    std::env::set_var("STK_FONTCACHE", dir.join("cache.json"));

    let mut used = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
    let requests = request_for(&used);
    let hits = evaluate_or_rescan(&mut used, &[fonts.as_path()], &[], &requests).unwrap();
    let again = {
        let mut db = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Use);
        assert_eq!(db.cache, CacheState::Hit);
        let m = evaluate_or_rescan(&mut db, &[fonts.as_path()], &[], &requests).unwrap();
        identify(&db, &m, &[])
    };
    // 明确关掉缓存的那条路：既不该留下缓存文件，也该给同一个身份
    std::env::remove_var("STK_FONTCACHE");
    let mut off = FontDb::scan_cached(&[fonts.as_path()], &[], CacheMode::Off);
    assert_eq!(off.cache, CacheState::Disabled);
    let off_matches = evaluate_or_rescan(&mut off, &[fonts.as_path()], &[], &requests).unwrap();
    let off_env = identify(&off, &off_matches, &[]);

    let first = identify(&used, &hits, &[]);
    assert_eq!(first.digest, again.digest, "命中与首扫必须同一身份");
    assert_eq!(first.digest, off_env.digest, "--no-font-cache 不该换身份");
    std::fs::remove_dir_all(&dir).ok();
}
