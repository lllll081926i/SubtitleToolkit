//! 37 §2 磁盘渲染缓存的**端到端**行为：走真 CLI、真 libass 渲染。
//!
//! 单元测试（`cli/src/rendercache.rs`）验的是编码器与配额；这一组验的是
//! 那层缓存被接到渲染路径上之后，**用户看得见的行为**是否正确：
//!
//! 1. 第二次运行真的命中，且产物逐字节不变；
//! 2. `--no-render-cache` 给出同样的产物（缓存不改变输出，只改变耗时）；
//! 3. 输入内容一变就重渲染（绝不把旧像素当新结果）；
//! 4. `cache stat/clear` 报的数与磁盘上的实际情况一致。
//!
//! 只测「可判定的东西」：不测 "更快"（那需要基准，机器噪声会让它假红），
//! 测的是 `render_cache.hit` 这个显式事实 —— 它是命令自己报出来的，可以核对。

use std::path::{Path, PathBuf};
use std::process::Command;

fn dll() -> Option<PathBuf> {
    let p = std::env::var("STK_LIBASS_DLL").ok()?;
    let p = PathBuf::from(p);
    p.is_file().then_some(p)
}

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rc-e2e-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,30,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.00,D,,0,0,0,,Hello\n";

fn render(
    dir: &Path,
    cache_dir: &Path,
    ass: &Path,
    out: &Path,
    extra: &[&str],
) -> serde_json::Value {
    let mut c = Command::new(cli());
    c.arg("render")
        .arg(ass)
        .args(["--fps", "25", "--canvas", "640x360"])
        .arg("--out")
        .arg(out)
        .arg("--json")
        .args(extra)
        .env("STK_LIBASS_DLL", dll().unwrap())
        .env("STK_RENDERCACHE_DIR", cache_dir)
        .current_dir(dir);
    let o = c.output().expect("跑 CLI");
    let stdout = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "输出不是单个 JSON 信封：{e}\nstdout={stdout}\nstderr={}",
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn cache_of(v: &serde_json::Value) -> (bool, bool, u64) {
    let c = &v["data"]["render_cache"];
    (
        c["enabled"].as_bool().unwrap_or(false),
        c["hit"].as_bool().unwrap_or(false),
        c["counters"]["hits"].as_u64().unwrap_or(0),
    )
}

fn sha(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let b = std::fs::read(p).unwrap();
    Sha256::digest(&b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("png"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// 第二次运行命中缓存，产物与第一次逐字节相同（XML 与每个 PNG）。
#[test]
fn a_second_run_hits_the_disk_cache_and_reproduces_the_same_bytes() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL（设 STK_LIBASS_DLL）");
        return;
    };
    let w = scratch("hit");
    let cache = w.join("cache");
    let ass = w.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();

    let v1 = render(&w, &cache, &ass, &w.join("r1").join("a.xml"), &[]);
    let (en1, hit1, _) = cache_of(&v1);
    assert!(en1, "缓存该是默认开着的：{v1}");
    assert!(!hit1, "第一次不可能命中：{v1}");

    let v2 = render(&w, &cache, &ass, &w.join("r2").join("a.xml"), &[]);
    let (_, hit2, hits2) = cache_of(&v2);
    assert!(hit2, "第二次该命中磁盘缓存：{v2}");
    assert!(hits2 >= 1, "命中计数该涨：{v2}");

    let x1 = w.join("r1").join("a.xml");
    let x2 = w.join("r2").join("a.xml");
    assert_eq!(sha(&x1), sha(&x2), "命中与否不该改变 XML 字节");

    let p1 = pngs(&w.join("r1"));
    let p2 = pngs(&w.join("r2"));
    assert!(!p1.is_empty(), "该有 PNG 产物");
    assert_eq!(p1.len(), p2.len(), "PNG 个数该一致");
    for (a, b) in p1.iter().zip(p2.iter()) {
        assert_eq!(sha(a), sha(b), "命中与否不该改变位图字节：{a:?} vs {b:?}");
    }
}

/// `--no-render-cache` 给出与命中完全相同的产物。
/// 这是"缓存是纯优化"这句声明的可核对形式 —— 若两者不同，说明缓存改变了结果。
#[test]
fn disabling_the_cache_gives_byte_identical_output() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL（设 STK_LIBASS_DLL）");
        return;
    };
    let w = scratch("nocache");
    let cache = w.join("cache");
    let ass = w.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();

    render(&w, &cache, &ass, &w.join("r1").join("a.xml"), &[]);
    let v_hit = render(&w, &cache, &ass, &w.join("r2").join("a.xml"), &[]);
    assert!(cache_of(&v_hit).1, "前提：这一跑该命中：{v_hit}");

    let v_off = render(
        &w,
        &cache,
        &ass,
        &w.join("r3").join("a.xml"),
        &["--no-render-cache"],
    );
    let (en, hit, _) = cache_of(&v_off);
    assert!(!en, "关掉后 enabled 该是 false（报告要说实话）：{v_off}");
    assert!(!hit, "关掉后不该报命中：{v_off}");

    assert_eq!(
        sha(&w.join("r2").join("a.xml")),
        sha(&w.join("r3").join("a.xml")),
        "命中与否必须给出同一份产物"
    );
    let a = pngs(&w.join("r2"));
    let b = pngs(&w.join("r3"));
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(sha(x), sha(y), "位图必须一致：{x:?} vs {y:?}");
    }
}

/// 输入一变就必须重渲染。这条是全部缓存设计里最危险的一格：
/// key 少一项就会把旧像素当成新结果交出去，而产物看起来完全正常。
#[test]
fn a_changed_input_is_re_rendered_instead_of_serving_stale_pixels() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL（设 STK_LIBASS_DLL）");
        return;
    };
    let w = scratch("stale");
    let cache = w.join("cache");
    let ass = w.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();

    render(&w, &cache, &ass, &w.join("r1").join("a.xml"), &[]);
    let before = sha(&w.join("r1").join("a.xml"));

    // 改文案（同一路径、长度也刻意相同：只靠"文件变了"是不够的，
    // 内容哈希才是判据）
    std::fs::write(&ass, ASS.replace("Hello", "World")).unwrap();
    let v = render(&w, &cache, &ass, &w.join("r2").join("a.xml"), &[]);
    assert!(!cache_of(&v).1, "改了内容就不该命中：{v}");

    let after = sha(&w.join("r2").join("a.xml"));
    assert_ne!(before, after, "不同文案该产出不同 XML");
    let a = pngs(&w.join("r1"));
    let b = pngs(&w.join("r2"));
    assert_eq!(a.len(), b.len(), "PNG 个数：{a:?} vs {b:?}");
    assert!(
        a.iter().zip(b.iter()).any(|(x, y)| sha(x) != sha(y)),
        "至少一张位图该不同，否则就是拿了陈旧像素"
    );

    // 改回去应重新命中（内容哈希是判据，不是"曾经见过"）
    std::fs::write(&ass, ASS).unwrap();
    let v2 = render(&w, &cache, &ass, &w.join("r3").join("a.xml"), &[]);
    assert!(cache_of(&v2).1, "内容改回原样该重新命中：{v2}");
    assert_eq!(
        sha(&w.join("r1").join("a.xml")),
        sha(&w.join("r3").join("a.xml"))
    );
}

/// `cache stat` / `cache clear` 报的数必须与磁盘实际情况一致。
/// 一个报错数的缓存工具比没有更糟：用户会按它做决定。
#[test]
fn cache_stat_and_clear_report_the_actual_disk_state() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL（设 STK_LIBASS_DLL）");
        return;
    };
    let w = scratch("stat");
    let cache = w.join("cache");
    let ass = w.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    render(&w, &cache, &ass, &w.join("r1").join("a.xml"), &[]);

    let out = Command::new(cli())
        .args(["cache", "stat", "--json"])
        .env("STK_RENDERCACHE_DIR", &cache)
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    let d = &v["data"];
    let reported_n = d["entries"].as_u64().unwrap();
    let reported_b = d["bytes"].as_u64().unwrap();

    let real: Vec<_> = std::fs::read_dir(&cache)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("rc"))
        .collect();
    assert_eq!(reported_n as usize, real.len(), "条数不符：{v}");
    let real_bytes: u64 = real.iter().map(|e| e.metadata().unwrap().len()).sum();
    assert_eq!(reported_b, real_bytes, "字节数不符：{v}");
    assert!(reported_b > 0, "该有内容：{v}");
    assert_eq!(d["dir"].as_str().unwrap(), cache.display().to_string());

    let out = Command::new(cli())
        .args(["cache", "clear", "--json"])
        .env("STK_RENDERCACHE_DIR", &cache)
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(
        v["data"]["removed_entries"].as_u64().unwrap() as usize,
        real.len()
    );
    assert_eq!(v["data"]["entries_after"].as_u64().unwrap(), 0);
    assert_eq!(v["data"]["bytes_after"].as_u64().unwrap(), 0);
    let left = std::fs::read_dir(&cache)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("rc"))
        .count();
    assert_eq!(left, 0, "clear 之后该一条不剩");
}

/// 环境变量 `STK_RENDERCACHE=0` 关掉这一层，并且报告要如实说"关了"。
#[test]
fn the_environment_can_turn_the_layer_off_and_the_report_says_so() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL（设 STK_LIBASS_DLL）");
        return;
    };
    let w = scratch("envoff");
    let cache = w.join("cache");
    let ass = w.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = w.join("r1").join("a.xml");

    let o = Command::new(cli())
        .arg("render")
        .arg(&ass)
        .args(["--fps", "25", "--canvas", "640x360", "--out"])
        .arg(&out)
        .arg("--json")
        .env("STK_LIBASS_DLL", dll().unwrap())
        .env("STK_RENDERCACHE_DIR", &cache)
        .env("STK_RENDERCACHE", "0")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    let (en, hit, _) = cache_of(&v);
    assert!(!en, "被环境变量关掉时 enabled 该是 false：{v}");
    assert!(!hit, "{v}");
    let n = std::fs::read_dir(&cache).map(|rd| rd.count()).unwrap_or(0);
    assert_eq!(n, 0, "关掉之后不该往磁盘写任何东西");
}
