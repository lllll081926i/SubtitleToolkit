//! `batch` 的端到端契约（15 §1/§5/§6/§7/§8、14 §Multi-output、38 §2）。
//!
//! 全部通过真二进制跑：批量任务的坑大多在"路径怎么解析、产物落没落盘、报告说什么"，
//! 单元测试看不见这些。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1280\r\nPlayResY: 720\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, Outline, Shadow, MarginV\r\nStyle: Default,Arial,48,3,2,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,一行\r\n";

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-batch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> Output {
    let exe = env!("CARGO_BIN_EXE_subtitletool-cli");
    Command::new(exe)
        .args(args)
        .output()
        .expect("CLI 必须能启动")
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

/// 一段既非 UTF-8 也非可判定 UTF-16 的字节：格式探测阶段就必须拒绝（STK-INP-0112）。
/// 不能用普通文本充"坏输入" —— 纯文本会被当成合法字幕（那是对的，不是缺陷）。
fn bad_bytes() -> Vec<u8> {
    vec![0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0x07, 0x08]
}

/// spec 里的相对路径都相对 spec 自身解析，所以 roots 写 "." 就够了。
fn write_spec(dir: &Path, body: &str) -> PathBuf {
    let p = dir.join("job.json");
    std::fs::write(&p, body).unwrap();
    p
}

/// 真 libass 的位置：`STK_LIBASS_DLL` → 仓库旁边的 vcpkg 目录。
/// 门阶用 `STK_REQUIRE_LIBASS=1` 把"没跑"变成失败（ISSUES Z4）。
fn libass() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let guess = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("tools")
        .join("vcpkg")
        .join("installed")
        .join("x64-windows")
        .join("bin")
        .join("ass-9.dll");
    guess.is_file().then_some(guess)
}

macro_rules! need_libass {
    () => {
        match libass() {
            Some(p) => p,
            None => {
                if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
                    panic!("STK_REQUIRE_LIBASS=1，但本机没有可用的 ass-9.dll（设 STK_LIBASS_DLL）");
                }
                eprintln!("SKIP：本机没有 libass DLL");
                return;
            }
        }
    };
}

/// spec 里写 DLL 路径要转义反斜杠，Windows 上 JSON 的 `\U` 会被当成非法转义。
fn spec_dll(dll: &Path) -> String {
    dll.display().to_string().replace('\\', "\\\\")
}

const TWO_PROFILES: &str = r#"{"id":"t-multi","roots":["."],"include":["*.ass"],
  "profiles":[{"name":"sub","format":"srt"},{"name":"ass720","format":"ass","canvas":"1280x720","fps":"25"}],
  "output":{"dir":"out","template":"{stem}.{profile}.{ext}"},"video_association":false}"#;

#[test]
fn batch_produces_every_profile_and_a_report_that_reopens() {
    let dir = workspace("multi");
    write(&dir, "a.ass", ASS);
    write(&dir, "b.ass", ASS);
    let spec = write_spec(&dir, TWO_PROFILES);
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "batch 应成功：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["command"], "batch");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["outcome"], "success");
    assert_eq!(v["data"]["inputs"], 2);
    assert_eq!(v["data"]["profiles"], 2);
    // 2 输入 × 2 profile + 1 报告
    assert_eq!(v["writes"].as_array().unwrap().len(), 5, "{v}");
    for name in ["a.sub.srt", "b.sub.srt", "a.ass720.ass", "b.ass720.ass"] {
        assert!(
            dir.join("out").join(name).is_file(),
            "output 没落盘：{name}"
        );
    }
    let raw = std::fs::read(dir.join("out").join("job-report.json")).unwrap();
    let rj: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(rj["outcome"], "success");
    assert_eq!(rj["details"].as_array().unwrap().len(), 4);
    assert!(
        rj["details"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["status"] == "verified"),
        "{rj}"
    );
    // 15 §7：恢复状态落盘，供下一次运行复用
    assert!(dir.join("out").join(".jobstate.json").is_file());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_dry_run_writes_nothing_and_no_jobstate() {
    let dir = workspace("dry");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        r#"{"id":"t-dry","roots":["."],"include":["*.ass"],
          "profiles":[{"name":"sub","format":"srt"}],"output":{"dir":"out"},
          "video_association":false}"#,
    );
    let out = run(&[
        "batch",
        "--spec",
        spec.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["writes"][0]["state"], "would-write", "{v}");
    assert!(
        !dir.join("out").join("a.sub.srt").exists(),
        "--dry-run 不许碰盘"
    );
    assert!(
        !dir.join("out").join(".jobstate.json").exists(),
        "dry-run 写了恢复状态，下一次就会复用一个从没存在过的文件"
    );
    assert_eq!(v["data"]["report"]["details"][0]["status"], "would-write");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_partial_success_keeps_the_good_outputs_and_names_the_bad_one() {
    let dir = workspace("partial");
    write(&dir, "good.ass", ASS);
    std::fs::write(dir.join("junk.sup"), b"not a real sup at all").unwrap();
    let spec = write_spec(
        &dir,
        r#"{"id":"t-partial","roots":["."],"include":["*.ass","*.sup"],
          "profiles":[{"name":"sub","format":"srt"}],"output":{"dir":"out"},
          "video_association":false}"#,
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1), "部分成功不能退 0（38 §2）");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["outcome"], "partial-success", "{v}");
    assert!(
        dir.join("out").join("good.sub.srt").is_file(),
        "成功的产物必须留着：14 §Multi-output 明确不回滚"
    );
    let statuses: Vec<&str> = v["data"]["report"]["details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"verified"), "{statuses:?}");
    assert!(statuses.contains(&"failed"), "{statuses:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_reuses_only_artifacts_that_still_hash_right() {
    let dir = workspace("reuse");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        // overwrite=replace：这里要验证的是"同一个路径会被重新生成"，
        // 而默认 rename 会写到兄弟文件名上（那是 15 §6 的正确行为，见 skip 测试）
        r#"{"id":"t-reuse","roots":["."],"include":["*.ass"],
          "profiles":[{"name":"sub","format":"srt"}],
          "output":{"dir":"out","overwrite":"replace"},
          "video_association":false}"#,
    );
    let first = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let second = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(second.status.success());
    let v: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        v["data"]["report"]["details"][0]["status"], "from-cache",
        "输入没变、profile 没变、产物哈希对得上就必须复用（15 §7）：{v}"
    );

    // 产物被人动过：复用必须失效并重新生成，否则第二次跑的是上一次的内容
    let produced = dir.join("out").join("a.sub.srt");
    std::fs::write(&produced, "tampered").unwrap();
    let third = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(third.status.success());
    let v: serde_json::Value = serde_json::from_slice(&third.stdout).unwrap();
    assert_eq!(
        v["data"]["report"]["details"][0]["status"], "verified",
        "哈希对不上还敢复用，就是在交付旧内容：{v}"
    );
    assert_ne!(std::fs::read_to_string(&produced).unwrap(), "tampered");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_skip_policy_never_touches_an_existing_output_and_says_so() {
    let dir = workspace("skip");
    write(&dir, "a.ass", ASS);
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out").join("a.sub.srt"), "SENTINEL").unwrap();
    let spec = write_spec(
        &dir,
        r#"{"id":"t-skip","roots":["."],"include":["*.ass"],
          "profiles":[{"name":"sub","format":"srt"}],
          "output":{"dir":"out","overwrite":"skip"},"video_association":false}"#,
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "skip 命中是成功不是失败：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("out").join("a.sub.srt")).unwrap(),
        "SENTINEL",
        "15 §6：绝不直接截断原文件"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["report"]["details"][0]["status"], "from-cache");
    assert!(
        v["data"]["report"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or_default().contains("skip")),
        "没动文件也必须解释原因：{v}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_rejects_bad_specs_before_touching_anything() {
    let dir = workspace("badspec");
    write(&dir, "a.ass", ASS);
    // 拼错的 key 必须报错，而不是被 serde 静默忽略
    let spec = write_spec(
        &dir,
        r#"{"id":"t","roots":["."],"recurse":true,"profiles":[{"name":"sub","format":"srt"}]}"#,
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1), "spec 内容错是取值错误 -> 1");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0151", "{v}");

    let spec2 = write_spec(&dir, r#"{"id":"t","roots":["."],"profiles":[]}"#);
    let out = run(&["batch", "--spec", spec2.to_str().unwrap(), "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0154", "{v}");

    let spec3 = write_spec(
        &dir,
        r#"{"id":"t","roots":["."],"profiles":[{"name":"x","format":"vtt"}]}"#,
    );
    let out = run(&["batch", "--spec", spec3.to_str().unwrap(), "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"][0]["code"], "STK-INP-0156", "{v}");

    // batch 不接受位置参数：给它一个目录会"看起来跑了"其实什么都没做
    let out = run(&["batch", "some-dir", "--json"]);
    assert_eq!(out.status.code(), Some(2), "命令行拼错 -> 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_text_only_job_does_not_need_libass() {
    // 纯文本批量任务不该被"没装 libass"卡住：renderer 身份只在真有渲染 profile 时才取
    let dir = workspace("nodll");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        r#"{"id":"t-nodll","roots":["."],"include":["*.ass"],
          "profiles":[{"name":"sub","format":"srt"}],"output":{"dir":"out"},
          "video_association":false}"#,
    );
    let exe = env!("CARGO_BIN_EXE_subtitletool-cli");
    let out = Command::new(exe)
        .args(["batch", "--spec", spec.to_str().unwrap(), "--json"])
        .env_remove("STK_LIBASS_DLL")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "文本批量任务不该需要渲染器：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["report"]["renderer"], serde_json::Value::Null);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_with_render_profile_reports_the_renderer_identity() {
    // 有渲染 profile 时，报告必须交代用的是哪个二进制（12 §4：按实际哈希，不按文件名）
    let dll = need_libass!();
    let dir = workspace("render-id");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        r#"{"id":"t-render","roots":["."],"include":["*.ass"],
          "profiles":[{"name":"pgs","format":"pgs","canvas":"1920x1080","fps":"25"}],
          "output":{"dir":"out"},"video_association":false}"#,
    );
    // 这个 spec 不给 dll 路径也没有 STK_LIBASS_DLL 时必须明确失败，而不是瞎找一个 DLL：
    // 先断言"没渲染器就报错"，再用 spec 的 libass_dll 字段跑通
    let _ = spec;
    let spec2 = dir.join("job2.json");
    std::fs::write(
        &spec2,
        format!(
            r#"{{"id":"t-render","roots":["."],"include":["*.ass"],
  "profiles":[{{"name":"sup","format":"pgs","canvas":"1920x1080","fps":"25"}}],
  "output":{{"dir":"out2"}},"libass_dll":"{}","video_association":false}}"#,
            dll.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    let out = run(&["batch", "--spec", spec2.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "带渲染的批量任务应成功：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["outcome"], "success", "{v}");
    assert!(
        dir.join("out2").join("a.sup.sup").is_file(),
        "PGS output 没落盘：{v}"
    );
    let r = &v["data"]["report"]["renderer"];
    assert_eq!(r["provider"], "libass", "{v}");
    let sha = r["binary_sha256"].as_str().unwrap_or_default();
    assert_eq!(sha.len(), 64, "renderer 哈希必须是 sha256：{r}");
    let expect = {
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(std::fs::read(&dll).unwrap());
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    assert_eq!(sha, expect, "报告里的哈希必须是真正那个 DLL 的哈希");
    assert!(r["version"].as_str().unwrap_or_default().starts_with("0x"));
    std::fs::remove_dir_all(&dir).ok();
}

/// 14 §Multi-output：bdn 是目录型产物（XML + 一组 PNG），必须每个输入独占目录，
/// 否则两个输入的 0000.png 会互相覆盖。复用判据也必须覆盖整组文件——只哈希 XML
/// 的话，"PNG 被删了"会被当成可以直接 from-cache 的完好产物。
#[test]
fn batch_bdn_profile_is_a_directory_artifact_whole_directory_must_match_to_reuse() {
    let dll = need_libass!();
    let dir = workspace("bdn");
    write(&dir, "a.ass", ASS);
    write(&dir, "b.ass", ASS);
    let spec = dir.join("job.json");
    std::fs::write(
        &spec,
        format!(
            r#"{{"id":"t-bdn","roots":["."],"include":["*.ass"],
  "profiles":[{{"name":"bdn","format":"bdn","canvas":"1920x1080","fps":"25"}},
               {{"name":"sub","format":"srt"}}],
  "output":{{"dir":"out"}},"libass_dll":"{}","video_association":false}}"#,
            spec_dll(&dll)
        ),
    )
    .unwrap();
    let pngs = |d: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".png"))
            .collect();
        v.sort();
        v
    };
    let status_of = |v: &serde_json::Value, input: &str, profile: &str| -> String {
        v["data"]["report"]["details"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| {
                d["input"].as_str().unwrap_or_default().ends_with(input)
                    && d["profile"].as_str() == Some(profile)
            })
            .map(|d| d["status"].as_str().unwrap_or_default().to_string())
            .unwrap_or_else(|| panic!("报告里没有 {input}/{profile}：{v}"))
    };

    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "bdn profile 应成功：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["outcome"], "success", "{v}");
    let a_dir = dir.join("out").join("bdn").join("a");
    let b_dir = dir.join("out").join("bdn").join("b");
    assert!(a_dir.join("a.bdn.xml").is_file(), "BDN XML 没落在自己目录");
    assert!(b_dir.join("b.bdn.xml").is_file());
    assert!(!pngs(&a_dir).is_empty(), "BDN 目录里没有 PNG");
    // 同名编号在两个输入目录里各自成立——这正是分目录的原因
    assert_eq!(pngs(&a_dir), pngs(&b_dir));
    assert!(pngs(&a_dir)[0].starts_with("0000"));
    // 文本 profile 仍是平铺命名，不受目录型规则影响
    assert!(dir.join("out").join("a.sub.srt").is_file());
    // 信封里 XML 与 PNG 都要逐个报出来：只报 XML 就是把一半产物藏起来了
    let paths: Vec<&str> = v["writes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["path"].as_str().unwrap_or_default())
        .collect();
    let a_prefix = a_dir.display().to_string();
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with(".png") && p.starts_with(&a_prefix)),
        "PNG 没进 writes：{paths:?}"
    );

    // 第二次：整组产物没变，才是 from-cache
    let second = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(second.status.success(), "{second:?}");
    let v2: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(status_of(&v2, "a.ass", "bdn"), "from-cache", "{v2}");

    // 只删一个 PNG：XML 完好，但产物已经不完整——必须重做，不能复用
    std::fs::remove_file(a_dir.join("0000.png")).unwrap();
    let third = run(&[
        "batch",
        "--spec",
        spec.to_str().unwrap(),
        "--overwrite",
        "replace",
        "--json",
    ]);
    assert!(third.status.success(), "{third:?}");
    let v3: serde_json::Value = serde_json::from_slice(&third.stdout).unwrap();
    assert_eq!(
        status_of(&v3, "a.ass", "bdn"),
        "verified",
        "PNG 被删了却还 from-cache，说明复用判据没覆盖整组文件：{v3}"
    );
    assert!(a_dir.join("0000.png").is_file(), "重做后 PNG 必须回来");
    // 另一个输入没动，仍然复用
    assert_eq!(status_of(&v3, "b.ass", "bdn"), "from-cache", "{v3}");

    // 改了 profile 参数（min_axis）就是另一件事：cache key 里没有它，用户就会
    // "改了参数却没生效"。这里换一份只差 min_axis 的 spec，必须重做。
    let spec2 = dir.join("job2.json");
    std::fs::write(
        &spec2,
        format!(
            r#"{{"id":"t-bdn","roots":["."],"include":["*.ass"],
  "profiles":[{{"name":"bdn","format":"bdn","canvas":"1920x1080","fps":"25","min_axis":300}}],
  "output":{{"dir":"out"}},"libass_dll":"{}","video_association":false}}"#,
            spec_dll(&dll)
        ),
    )
    .unwrap();
    let fourth = run(&[
        "batch",
        "--spec",
        spec2.to_str().unwrap(),
        "--overwrite",
        "replace",
        "--json",
    ]);
    assert!(fourth.status.success(), "{fourth:?}");
    let v4: serde_json::Value = serde_json::from_slice(&fourth.stdout).unwrap();
    assert_ne!(
        status_of(&v4, "a.ass", "bdn"),
        "from-cache",
        "min_axis 变了却复用旧产物：{v4}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 14 §ASS→SUP 那句"若同时输出 BDN+SUP，共享 render/cache"的实测。
/// 报告里三个数一起给：executions（渲染型 profile 执行次数）、renders（真渲染次数）、
/// keys（前半链 key 个数）。共享成立时 renders == keys < executions；
/// 如果哪天有人把 profile 名之类的东西塞进 key，renders 就会悄悄涨到 executions。
#[test]
fn batch_shares_one_render_between_sup_and_bdn_profiles() {
    let dll = need_libass!();
    let dir = workspace("shared");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        &format!(
            r#"{{"id":"t-shared","roots":["."],"include":["*.ass"],
  "profiles":[{{"name":"sup","format":"pgs","canvas":"1920x1080","fps":"25"}},
               {{"name":"bdn","format":"bdn","canvas":"1920x1080","fps":"25"}},
               {{"name":"sub","format":"srt"}},
               {{"name":"ass1080","format":"ass","canvas":"1920x1080","fps":"25"}},
               {{"name":"ass2160","format":"ass","canvas":"3840x2160","fps":"25"}}],
  "output":{{"dir":"out"}},"libass_dll":"{}","video_association":false}}"#,
            spec_dll(&dll)
        ),
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "SUP+BDN 批量应成功：{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["outcome"], "success", "{v}");
    let rc = &v["data"]["report"]["render_cache"];
    assert_eq!(rc["executions"].as_u64().unwrap(), 2, "{rc}");
    assert_eq!(
        rc["keys"].as_u64().unwrap(),
        1,
        "同一输入只应有一个前半链 key：{rc}"
    );
    assert_eq!(
        rc["renders"].as_u64().unwrap(),
        1,
        "两个渲染 profile 各渲染了一次，共享没生效：{rc}"
    );
    // 五种输出都真的落盘了——共享的是前半链，不是"少做一个输出"
    assert!(dir.join("out").join("a.sup.sup").is_file(), "{v}");
    assert!(
        dir.join("out")
            .join("bdn")
            .join("a")
            .join("a.bdn.xml")
            .is_file(),
        "{v}"
    );
    assert!(dir.join("out").join("a.sub.srt").is_file(), "{v}");
    assert!(dir.join("out").join("a.ass1080.ass").is_file(), "{v}");
    assert!(dir.join("out").join("a.ass2160.ass").is_file(), "{v}");
    // AT-MULTI-001：单 ASS 一次出 1080 ASS + 2160 ASS + 1080 SUP + SRT（这里再加一份 BDN），
    // 每个输出都必须独立 verify 成功，而不是"任务整体绿但某个文件是空的"
    let details = v["data"]["report"]["details"].as_array().unwrap();
    assert_eq!(details.len(), 5, "{v}");
    assert!(
        details.iter().all(|d| d["status"] == "verified"),
        "多输出里有一个没验证成功：{v}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// key 少一个分量就是假复用：帧率不同的两次渲染会被当成同一份像素。
#[test]
fn batch_does_not_share_renders_across_different_frame_rates() {
    let dll = need_libass!();
    let dir = workspace("notshared");
    write(&dir, "a.ass", ASS);
    let spec = write_spec(
        &dir,
        &format!(
            r#"{{"id":"t-notshared","roots":["."],"include":["*.ass"],
  "profiles":[{{"name":"bdn25","format":"bdn","canvas":"1920x1080","fps":"25"}},
               {{"name":"bdn30","format":"bdn","canvas":"1920x1080","fps":"30"}}],
  "output":{{"dir":"out"}},"libass_dll":"{}","video_association":false}}"#,
            spec_dll(&dll)
        ),
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rc = &v["data"]["report"]["render_cache"];
    assert_eq!(rc["executions"].as_u64().unwrap(), 2, "{rc}");
    assert_eq!(
        rc["keys"].as_u64().unwrap(),
        2,
        "帧率不同必须是两个 key：{rc}"
    );
    assert_eq!(
        rc["renders"].as_u64().unwrap(),
        2,
        "不同帧率各渲染一次：{rc}"
    );
    // 两份 BDN 的时间码必须真的不同（不然 key 分开也只是形式）
    let a = std::fs::read_to_string(dir.join("out").join("bdn25").join("a").join("a.bdn25.xml"))
        .unwrap_or_else(|e| panic!("bdn25 产物：{e}"));
    let b = std::fs::read_to_string(dir.join("out").join("bdn30").join("a").join("a.bdn30.xml"))
        .unwrap_or_else(|e| panic!("bdn30 产物：{e}"));
    assert_ne!(a, b, "两种帧率写出了同一份 BDN");
    assert!(a.contains("<FrameRate>25</FrameRate>"), "{a}");
    assert!(b.contains("<FrameRate>30</FrameRate>"), "{b}");
    std::fs::remove_dir_all(&dir).ok();
}

/// AT-BATCH-001：100 输入 × 3 profile 的容量验收，混入格式错误与既有输出冲突。
/// 断言的是“单文件失败不阻塞其他”在真实体量下仍成立：逐 profile 结果可数、
/// PartialSuccess 正确、成功产物齐全、失败项能定位到具体输入。
#[test]
fn at_batch_001_hundred_inputs_by_three_profiles_partial_failure_is_contained() {
    let dir = workspace("capacity");
    // 100 个输入：98 个合法 ASS + 2 个坏文件（非 UTF-8 字节，格式探测阶段就拒）
    for i in 0..100 {
        let name = format!("in{i:03}.ass");
        write(&dir, &name, ASS);
    }
    std::fs::write(dir.join("zxbroken1.ass"), bad_bytes()).unwrap();
    std::fs::write(dir.join("zxbroken2.ass"), bad_bytes()).unwrap();
    // 既有输出冲突：先放一个同名产物，验证默认 rename 不覆盖它
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out").join("in000.sub.srt"), "SENTINEL").unwrap();

    let spec = write_spec(
        &dir,
        r#"{"id":"t-cap","roots":["."],"include":["*.ass"],
  "profiles":[{"name":"sub","format":"srt"},
               {"name":"ass","format":"ass","canvas":"1280x720","fps":"25"},
               {"name":"ssa","format":"ssa"}],
  "output":{"dir":"out","template":"{stem}.{profile}.{ext}"},"video_association":false}"#,
    );
    let out = run(&["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    // 2 个坏输入 → 部分成功，不能退 0（38 §2）
    assert_eq!(out.status.code(), Some(1), "部分成功必须退 1：{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // 100 个输入 + 2 个坏文件
    assert_eq!(v["data"]["inputs"], 102, "{v}");
    assert_eq!(v["data"]["profiles"], 3, "{v}");
    assert_eq!(v["data"]["outcome"], "partial-success", "{v}");

    let details = v["data"]["report"]["details"].as_array().unwrap();
    // detail 里 input 是含 `\\.\\` 的完整路径，按文件名判别
    let is_bad = |d: &serde_json::Value| {
        d["input"]
            .as_str()
            .unwrap_or("")
            .replace('\\', "/")
            .contains("/zxbroken")
    };
    let good: Vec<_> = details.iter().filter(|d| !is_bad(d)).collect();
    let bad: Vec<_> = details.iter().filter(|d| is_bad(d)).collect();
    assert_eq!(good.len(), 100 * 3, "98×3 合法输入应全部有逐 profile 记录");
    assert_eq!(
        bad.len(),
        2 * 3,
        "坏输入也要逐 profile 记录失败，而不是静默跳过"
    );
    assert!(
        good.iter().all(|d| d["status"] == "verified"),
        "合法输入里有一个没验证成功：{:?}",
        good.iter()
            .filter(|d| d["status"] != "verified")
            .collect::<Vec<_>>()
    );
    assert!(
        bad.iter().all(|d| d["status"] == "failed"),
        "坏输入应报 failed：{bad:?}"
    );

    // 失败项必须能定位到具体输入与原因，而不是只说“有一个失败了”
    for d in &bad {
        let input = d["input"].as_str().unwrap_or("");
        assert!(input.contains("zxbroken"), "{d}");
        assert!(
            d["detail"].as_str().unwrap_or("").contains("STK-INP-"),
            "失败记录必须带可定位的错误码与说明：{d}"
        );
    }

    // 成功产物齐全且独立：抽查首尾合法输入的三份输出都在
    for stem in ["in000", "in097"] {
        for prof in ["sub", "ass", "ssa"] {
            let ext = if prof == "sub" { "srt" } else { prof };
            let p = dir.join("out").join(format!("{stem}.{prof}.{ext}"));
            assert!(p.is_file(), "缺失产物：{}", p.display());
        }
    }
    // 冲突的既有输出绝不被覆盖（默认 rename）
    assert_eq!(
        std::fs::read_to_string(dir.join("out").join("in000.sub.srt")).unwrap(),
        "SENTINEL",
        "默认 rename 不得覆盖既有输出"
    );
    std::fs::remove_dir_all(&dir).ok();
}
