//! 任务执行：把 GUI 的输入集与 profile 矩阵交给**真实的** `cli::batch::run_batch`。
//!
//! ## 之前这里是什么
//!
//! 这里曾经是 `std::thread::sleep(50ms)` 加一句 `Ok(())`，注释写着"模拟处理或调用
//! task-engine 输出事务"。那是最坏的一种状态：任务抽屉会一个个变绿，用户以为
//! 字幕导出了，而磁盘上什么都没有。占位实现必须明确报错（20 §Commands 的同一条纪律），
//! 不能"看起来成功了"。
//!
//! ## 现在怎么走
//!
//! 1. 用 [`super::spec::compile_spec`] 编译出 `cli::batch::Spec`；
//! 2. 把它写成一个临时 spec 文件（`run_batch` 的入口就是文件路径，与 CLI 完全一致）；
//! 3. 调用 `cli::batch::run_batch`——**与 `subtitletool-cli batch` 同一份实现**；
//! 4. 把信封里的 `report.details` 逐行翻译成任务抽屉的状态。
//!
//! 第 2 步不是多余的：`run_batch` 收的是 spec 路径，而 spec 的相对路径也是相对它解析的。
//! 把"存盘"这一步做成真的，用户就能拿到那份 spec 自己去 CLI 重放（20 §Repro）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cli::args::{Command, Invocation};
use cli::batch::Spec;
use task_engine::OverwritePolicy;

/// 执行结果的逐条通报。
#[derive(Clone, Debug)]
pub enum Progress {
    /// 已经写好 spec，开始跑。带上 spec 路径与"做不到"的说明。
    Started {
        spec_path: PathBuf,
        notes: Vec<String>,
    },
    /// 某个 (输入, profile) 的最新状态。`status` 直接取自 CLI 报告，不重新解释。
    Row {
        input: String,
        profile: String,
        status: String,
        detail: String,
    },
    /// 整体判定（success / partial-success / failed / cancelled）
    Finished {
        outcome: String,
        report_path: Option<PathBuf>,
        error: Option<(String, String, String)>,
    },
}

/// 跑一次批量任务。**同步阻塞**，由调用方放进 `spawn_blocking`。
///
/// 返回 `Err` 只用于"任务根本起不来"（spec 编译/写盘失败）；
/// 单个 profile 的失败走 [`Progress::Row`]，与 CLI 的语义一致
/// （14 §Multi-output：一个 profile 坏了不回滚别的）。
pub fn run(
    spec: &Spec,
    output_dir: &Path,
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(Progress),
) -> Result<(), (String, String, String)> {
    tracing::info!(
        job_id = %spec.id,
        input_count = spec.roots.len(),
        profile_count = spec.profiles.len(),
        output_dir = %output_dir.display(),
        "GUI 批处理开始"
    );
    let job_id = spec.id.clone();
    let spec_path = output_dir.join(format!("{job_id}.spec.json"));
    let text = super::spec::spec_to_json(spec);
    if let Err(e) = std::fs::create_dir_all(output_dir) {
        return Err((
            "STK-OUT-0001".into(),
            "无法创建输出目录".into(),
            format!("{}：{e}", output_dir.display()),
        ));
    }
    if let Err(e) = std::fs::write(&spec_path, &text) {
        return Err((
            "STK-OUT-0002".into(),
            "无法写出 batch spec".into(),
            format!("{}：{e}", spec_path.display()),
        ));
    }
    emit(Progress::Started {
        spec_path: spec_path.clone(),
        notes: Vec::new(),
    });

    if cancel.load(Ordering::Relaxed) {
        emit(Progress::Finished {
            outcome: "cancelled".into(),
            report_path: None,
            error: None,
        });
        return Ok(());
    }

    // 与 `subtitletool-cli batch --spec X --json` 完全同一条路：
    // 同一个 Invocation、同一个 run_batch。命令行与 GUI 的区别只到这里为止。
    let inv = Invocation {
        command: Command::Batch {
            spec: spec_path.clone(),
        },
        json: true,
        out: None,
        dry_run: false,
        overwrite: OverwritePolicy::Rename,
        no_font_cache: false,
        inject_fonts: false,
        // GUI 的任务也受益于 37 §2 的磁盘缓存；关掉它没有理由。
        render_cache: true,
    };

    // 把 GUI 的取消标志交给引擎：它会在每个 (输入, profile) 之前检查，
    // 于是“取消”真的能拦住后面的格子（AT-UI-001）。
    let cancel_token = task_engine::CancelToken::from_shared(cancel);
    let env =
        match cli::batch::run_batch_with_cancel(&inv, &spec_path, "batch", Some(&cancel_token)) {
            Ok(env) => env,
            Err(e) => {
                tracing::error!(
                    job_id = %job_id,
                    code = %e.code_string(),
                    title = %e.title,
                    details = ?e.details,
                    "GUI 批处理启动失败"
                );
                let triple = (
                    e.code_string(),
                    e.title.clone(),
                    e.user_action.clone().unwrap_or_default(),
                );
                emit(Progress::Finished {
                    outcome: "failed".into(),
                    report_path: None,
                    error: Some(triple.clone()),
                });
                return Err(triple);
            }
        };

    // 逐行翻译 CLI 报告。**不重新解释状态**：status 字符串原样透出，
    // 这样 GUI 显示的与 CLI 报告里的永远是同一个词。
    let report = env.data.as_ref().and_then(|d| d.get("report"));
    let details = report
        .and_then(|r| r.get("details"))
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    for row in details {
        let s = |k: &str| {
            row.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let input = s("input");
        let profile = s("profile");
        let status = s("status");
        let detail = s("detail");
        tracing::debug!(
            job_id = %job_id,
            input = %Path::new(&input).file_name().and_then(|name| name.to_str()).unwrap_or(""),
            profile = %profile,
            status = %status,
            detail = %detail,
            "GUI 批处理条目完成"
        );
        emit(Progress::Row {
            input,
            profile,
            status,
            detail,
        });
    }

    let outcome = env
        .data
        .as_ref()
        .and_then(|d| d.get("outcome"))
        .and_then(|v| v.as_str())
        .unwrap_or("failed")
        .to_string();
    tracing::info!(job_id = %job_id, outcome = %outcome, "GUI 批处理结束");
    // 报告里的 error 是"任务级判定不是成功"（比如 partial-success），
    // 转成抽屉里的失败说明；成功的任务这里是空的。
    let error = env
        .errors
        .first()
        .map(|e| (e.code.clone(), e.title.clone(), String::new()));
    emit(Progress::Finished {
        outcome,
        report_path: Some(output_dir.join("job-report.json")),
        error,
    });
    Ok(())
}

/// 任务的作业 id：稳定、可读、不含非法文件名字符。
///
/// 不用时间戳：同一个 GUI 会话里反复点"开始"应该覆盖同一份 spec 与报告，
/// 而不是在输出目录里堆出一串 `job-<纳秒>.spec.json`——那些文件没人看，只会越积越多。
pub fn job_id() -> String {
    "gui-batch".to_string()
}

/// GUI 没有逐 profile 的帧率控件，spec 里的 fps 用与 CLI 相同的默认值。
/// 公开出来是为了让测试能断言"GUI 的默认与 CLI 的默认一致"，而不是各写各的字面量。
pub fn default_fps() -> &'static str {
    "25"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::profile::ProfilesState;

    #[test]
    fn job_id_is_a_usable_and_stable_filename() {
        let id = job_id();
        assert!(!id.is_empty());
        assert!(
            !id.chars()
                .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')),
            "作业 id 会变成文件名，不能含非法字符：{id}"
        );
        assert_eq!(id, job_id(), "同一个会话里反复启动应该落在同一份 spec 上");
    }

    /// GUI 的默认帧率必须与 CLI 的默认一致。
    /// 两处各写一个字面量，改了一边就变成静默分歧——用户会看到"同样的参数、不同的输出"。
    #[test]
    fn default_fps_matches_the_cli_default() {
        let (spec, _) = super::super::spec::compile_spec(
            "j",
            &[PathBuf::from("a.ass")],
            &ProfilesState::new().enabled_profiles(),
            &PathBuf::from("out"),
            None,
            "{stem}.{ext}",
        );
        for p in &spec.profiles {
            assert_eq!(
                p.fps,
                default_fps(),
                "profile {} 的 fps 与 GUI 声明的默认不一致",
                p.name
            );
        }
    }

    /// 起不来时必须给出三件套（码/原因/怎么办），而不是默默不动。
    #[test]
    fn an_unwritable_output_dir_is_reported_as_a_start_failure() {
        let profiles = ProfilesState::new();
        let (spec, _) = super::super::spec::compile_spec(
            "j",
            &[PathBuf::from("a.ass")],
            &profiles.enabled_profiles(),
            // 用一个"存在但其实是文件"的路径当目录：create_dir_all 必然失败
            &PathBuf::from("."),
            None,
            "{stem}.{ext}",
        );
        let file_as_dir =
            std::env::temp_dir().join(format!("stk-gui-notdir-{}", std::process::id()));
        std::fs::write(&file_as_dir, b"x").unwrap();
        let mut got: Vec<Progress> = Vec::new();
        let r = run(&spec, &file_as_dir, Arc::new(AtomicBool::new(false)), |p| {
            got.push(p)
        });
        assert!(r.is_err(), "把文件当目录必须失败");
        let (code, _, _) = r.unwrap_err();
        assert_eq!(code, "STK-OUT-0001");
        assert!(
            matches!(got.as_slice(), []),
            "起不来就不该报 Started：{got:?}"
        );
        let _ = std::fs::remove_file(&file_as_dir);
    }
}
