//! 环境自检（顶部栏那个按钮）。
//!
//! ## 为什么要有这个模块
//!
//! 那个按钮以前只把 `show_doctor` 设成 `true`，而界面从来不读它
//! —— 点了没有任何反应。“按钮在但没反应”与“功能没做”在用户看来是同一件事，
//! 而前者更难查：用户会以为自己点错了。
//!
//! ## 与 CLI 的关系
//!
//! 这里直接调 `cli::run::doctor`，**不另写一份自检**。
//! 20 §Repro 的同一条理由：同一件事出现两套判定，就会在某个时刻给出两个答案。
//! GUI 只是把那份 JSON 摆出来。

use std::path::PathBuf;

/// 跑一次 self-check，返回 CLI 的 `doctor` 数据段。
///
/// 失败时返回**给人看的说明**而不是黑箱：拿不到自检结果本身就是要报告的事。
pub fn run() -> Result<serde_json::Value, String> {
    let inv = cli::args::Invocation {
        command: cli::args::Command::Doctor,
        json: true,
        out: None,
        dry_run: false,
        overwrite: task_engine::OverwritePolicy::Rename,
        no_font_cache: false,
        inject_fonts: false,
    };
    match cli::run::execute(&inv) {
        Ok(env) => Ok(env.data.unwrap_or(serde_json::Value::Null)),
        Err(e) => Err(format!("{}：{}", e.code_string(), e.title)),
    }
}

/// 把 doctor 的 data 抽成界面直接能显示的一组行。
///
/// 抽出来是为了让“显示什么”可以被测试断言——直接读 JSON 在 UI 里是写不出来的。
pub fn rows(data: &serde_json::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let s = |v: &serde_json::Value| match v {
        serde_json::Value::String(x) => x.clone(),
        serde_json::Value::Null => "（无）".to_string(),
        other => other.to_string(),
    };

    out.push(("操作系统".into(), s(&data["os"])));
    out.push(("架构".into(), s(&data["arch"])));
    out.push((
        "当前目录可写".into(),
        match data["cwd_writable"].as_bool() {
            Some(true) => "是".into(),
            Some(false) => "否（输出事务会失败）".into(),
            None => "未知".into(),
        },
    ));

    // ffprobe：有则报身份，无则说清“视频辅助不可用”而不是留空
    match &data["ffprobe"] {
        serde_json::Value::Null => out.push(("ffprobe".into(), "未找到（视频辅助不可用）".into())),
        v => {
            out.push(("ffprobe".into(), s(&v["path"])));
            let digest = s(&v["digest_sha256"]);
            if digest.len() > 20 {
                out.push(("ffprobe 摘要".into(), digest[..20].to_string()));
            }
        }
    }

    // render-host：28 §2 把它列为独立产物，没打包就是没有
    out.push((
        "render host".into(),
        match data["render_host"]["available"].as_bool() {
            Some(true) => {
                let p = s(&data["render_host"]["path"]);
                let prov = s(&data["render_host"]["handshake"]["provider"]);
                format!("{prov} @ {p}")
            }
            Some(false) => "不可用（渲染类命令会失败）".to_string(),
            None => "未知".to_string(),
        },
    ));

    // 数据目录（28 §4）：装在包里还是写在用户目录，是要让用户看见的
    let dd = &data["data_dir"];
    out.push((
        "数据目录".into(),
        format!("{}（{}）", s(&dd["path"]), s(&dd["kind"])),
    ));
    out.push(("数据目录理由".into(), s(&dd["why"])));

    // 未实现能力：doctor 说错就等于用户对功能面的判断错
    let nb = data["not_built"]
        .as_array()
        .map(|a| a.iter().map(&s).collect::<Vec<_>>().join("、"))
        .unwrap_or_default();
    out.push((
        "未实现命令".into(),
        if nb.is_empty() {
            "（无：已规划命令全部落地）".to_string()
        } else {
            nb
        },
    ));

    out
}

/// 渲染宿主可执行文件在哪：`STK_RENDERER_HOST` → 与 GUI 同目录的同名 exe。
/// 直接复用 CLI 的那份解析（一条规则只能有一处实现），
/// 否则“GUI 找不到 host”与“CLI 找不到 host”会给出两个不同答案。
pub fn renderer_host_exe() -> Option<PathBuf> {
    cli::run::renderer_host_exe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_cover_the_fields_the_doctor_button_is_supposed_to_show() {
        let data = serde_json::json!({
            "os": "windows",
            "arch": "x86_64",
            "cwd_writable": true,
            "ffprobe": null,
            "render_host": {"available": false},
            "data_dir": {"path": "D:\\p\\data", "kind": "portable", "why": "portable.flag 存在"},
            "not_built": []
        });
        let r = rows(&data);
        let keys: Vec<&str> = r.iter().map(|(k, _)| k.as_str()).collect();
        for want in [
            "操作系统",
            "架构",
            "当前目录可写",
            "ffprobe",
            "render host",
            "数据目录",
        ] {
            assert!(keys.contains(&want), "少了 {want}：{keys:?}");
        }
        // 拿不到的东西必须说明后果，不能留空
        let ff = r.iter().find(|(k, _)| k == "ffprobe").unwrap();
        assert!(ff.1.contains("不可用"), "{}", ff.1);
        let rh = r.iter().find(|(k, _)| k == "render host").unwrap();
        assert!(rh.1.contains("渲染"), "{}", rh.1);
        // 未实现清单为空时要明确说"没有"，而不是给一个空串
        let nb = r.iter().find(|(k, _)| k == "未实现命令").unwrap();
        assert!(nb.1.contains("无"), "{}", nb.1);
    }

    #[test]
    fn a_non_empty_not_built_list_is_shown_verbatim() {
        let data = serde_json::json!({"not_built": ["foo", "bar"]});
        let r = rows(&data);
        let nb = r.iter().find(|(k, _)| k == "未实现命令").unwrap();
        assert!(nb.1.contains("foo") && nb.1.contains("bar"), "{}", nb.1);
    }

    /// 自检必须真的跑得起来（至少在这个测试进程里）。
    #[test]
    fn doctor_runs_and_reports_a_real_environment() {
        let d = run().expect("doctor 不该失败");
        assert!(d["os"].is_string(), "{d}");
        assert!(d["arch"].is_string());
        assert!(d["not_built"].is_array(), "未实现清单必须是数组：{d}");
    }
}
