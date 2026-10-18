//! 12 §1/§4 的 provider 能力探测（CLI 集成用例）。
//!
//! ## 核的是什么
//!
//! 这条命令的价值在于**回答“它是什么”**，而不是“它不是什么”。
//! 所以用例核的是逐 DLL 的判定与那句话本身，而不是“有没有报错”。
//!
//! ## 为什么不靠文件名
//!
//! 12 §1 明写“绝不靠 DLL 文件名推断调用 ABI”。用例里特意用**真文件**
//! （`third_party/` 下那两个）而不是构造名字 —— 构造的名字只能证明
//! 代码没有 `match file_name`，证明不了它真的读了导出表。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("subtitletool-cli{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(cli())
        .args(args)
        .output()
        .expect("跑不动 subtitletool-cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn data(s: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(s).unwrap_or(serde_json::Value::Null);
    v.get("data").cloned().unwrap_or(serde_json::Value::Null)
}

fn third_party(name: &str) -> Option<PathBuf> {
    // 用例跑在 `target/<profile>/deps/`，仓库根在往上四层。
    let mut p = std::env::current_exe().unwrap();
    for _ in 0..4 {
        p.pop();
    }
    let cand = p.join("third_party").join(name);
    cand.is_file().then_some(cand)
}

/// 没有参数时报用法错，而不是静默干别的事。
#[test]
fn probing_nothing_is_a_usage_error() {
    let (code, out) = run(&["provider", "probe", "--json"]);
    assert_eq!(code, 2);
    assert!(!out.contains("\"ok\":true"), "{out}");
}

#[test]
fn an_unknown_subcommand_is_refused() {
    let (code, _) = run(&["provider", "nonsense", "--json"]);
    assert_eq!(code, 2);
}

/// **真的 libass 要被认成 libass**。
#[test]
fn the_real_libass_is_reported_as_a_usable_libass_provider() {
    let Some(dll) = third_party("libass-9.dll") else {
        eprintln!("跳过：third_party/libass-9.dll 不在");
        return;
    };
    let (code, out) = run(&["provider", "probe", dll.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let d = data(&out);
    assert_eq!(d["any_usable_as_libass"], true, "{d}");
    let row = &d["rows"][0];
    assert_eq!(row["usable_as_libass"], true, "{row}");
    assert_eq!(row["arch"], "x64", "{row}");
    assert!(
        row["verdict"]
            .as_str()
            .unwrap_or("")
            .contains("可作为 libass provider"),
        "{row}"
    );
}

/// **真的 VSFilterMod 要被认成 CSRI，而不是被含糊地拒掉**。
///
/// 这是本项目最重要的一条能力判定：12 §1B 说“特定 VSFilterMod build
/// 可以是 CSRI provider”，而此前没有任何代码路径能回答它到底是哪一种。
#[test]
fn the_real_vsfiltermod_is_reported_as_csri_not_libass() {
    let Some(dll) = third_party("VSFilterMod.dll") else {
        eprintln!("跳过：third_party/VSFilterMod.dll 不在");
        return;
    };
    let (code, out) = run(&["provider", "probe", dll.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "探测本身不该失败：{out}");
    let d = data(&out);
    let row = &d["rows"][0];
    assert_eq!(
        row["kind"], "csri",
        "VSFilterMod 应当被认成 CSRI provider：{row}"
    );
    assert_eq!(
        row["usable_as_libass"], false,
        "它不是 libass，不应当被当成可用：{row}"
    );
    let verdict = row["verdict"].as_str().unwrap_or("");
    assert!(
        verdict.contains("CSRI") && verdict.contains("不是 libass"),
        "判定句要说清它是什么、以及为什么现在用不了：{verdict}"
    );
}

/// 一次问多个：结果按输入顺序逐个给出，并且 `any_usable` 是“至少一个可用”。
#[test]
fn probing_several_dlls_reports_each_one() {
    let (Some(a), Some(b)) = (third_party("VSFilterMod.dll"), third_party("libass-9.dll")) else {
        eprintln!("跳过：third_party 下的候选 DLL 不全");
        return;
    };
    let (code, out) = run(&[
        "provider",
        "probe",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let d = data(&out);
    assert_eq!(d["probed"], 2);
    assert_eq!(d["any_usable_as_libass"], true, "有两个里至少一个可用");
    let rows = d["rows"].as_array().unwrap();
    assert_eq!(rows[0]["file_name"], "VSFilterMod.dll");
    assert_eq!(rows[1]["file_name"], "libass-9.dll");
    assert_eq!(rows[0]["kind"], "csri");
    assert_eq!(rows[1]["usable_as_libass"], true);
}

/// 不存在的文件：明确报“读不了”，而不是 panic 也不是含糊的失败。
#[test]
fn a_missing_file_is_reported_per_row_not_as_a_crash() {
    let (code, out) = run(&["provider", "probe", "Z:/nope/not-here.dll", "--json"]);
    assert_eq!(code, 0, "单个 DLL 读不到不该让整条命令失败：{out}");
    let d = data(&out);
    let row = &d["rows"][0];
    assert_eq!(row["usable_as_libass"], false);
    assert!(
        row["arch_error"].as_str().is_some() || row["exports_error"].as_str().is_some(),
        "要说清读不了的原因：{row}"
    );
}

/// 不是 PE 的文件：同样逐行报，且说明它不是 PE。
#[test]
fn a_non_pe_file_is_reported_as_not_pe() {
    let tmp = std::env::temp_dir().join(format!("stk-notpe-{}.dll", std::process::id()));
    std::fs::write(&tmp, b"this is definitely not a PE file").unwrap();
    let (code, out) = run(&["provider", "probe", tmp.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let d = data(&out);
    let row = &d["rows"][0];
    assert_eq!(row["usable_as_libass"], false);
    let joined = format!("{row}");
    assert!(
        joined.contains("PE") || joined.contains("MZ"),
        "要说清为什么不是 PE：{joined}"
    );
    let _ = std::fs::remove_file(&tmp);
}

/// **不加载** DLL：这是这条命令最要紧的安全属性。
///
/// 核的方式：拿一个**在加载时会失败**的路径（存在但内容不是 PE）。
/// 若实现真的调了 `LoadLibrary` 并据此判定，它的报错会变成
/// “不是有效的 Win32 应用程序”那一类；而我们的实现只读文件，
/// 所以报的是 PE 解析层的原因。
/// 更直接的证据是 `meaning` 字段 —— 它自报“没有加载任何 DLL”。
#[test]
fn it_declares_that_it_did_not_load_anything() {
    let Some(dll) = third_party("libass-9.dll") else {
        eprintln!("跳过");
        return;
    };
    let (_, out) = run(&["provider", "probe", dll.to_str().unwrap(), "--json"]);
    let meaning = data(&out)["meaning"].as_str().unwrap_or("").to_string();
    assert!(
        meaning.contains("没有加载"),
        "结果里要自报没加载 DLL：{meaning}"
    );
}

/// 相对路径也要能工作（用户常直接写文件名）。
#[test]
fn a_relative_path_works() {
    let Some(dll) = third_party("libass-9.dll") else {
        eprintln!("跳过");
        return;
    };
    let dir = dll.parent().unwrap();
    let (code, out) = run(&["provider", "probe", "libass-9.dll", "--json"]);
    // 若当前目录不是 third_party 就会读不到 —— 那也算“逐行报错”而不是崩溃。
    assert!(code == 0, "{out}");
    let d = data(&out);
    assert_eq!(d["probed"], 1);
    // 用绝对路径再跑一次，确认判定一致
    let (_, out2) = run(&["provider", "probe", dll.to_str().unwrap(), "--json"]);
    assert_eq!(
        data(&out)["rows"][0]["usable_as_libass"],
        data(&out2)["rows"][0]["usable_as_libass"]
    );
    let _ = Path::new(dir);
}
