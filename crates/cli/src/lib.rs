//! subtitletool-cli 入口（20_CLI_AUTOMATION.md）。
//!
//! stdout 只放结果（`--json` 时是一行 JSONL 信封，否则是人读摘要）；
//! 进度与诊断一律走 stderr，这样 `> out.json` 不会被污染。

pub mod args;
pub mod batch;
pub mod defaults;
pub mod deliver;
pub mod diagnostic;
pub mod font;
pub(crate) mod fontenv;
pub(crate) mod front;
pub mod mkv;
pub mod output;
pub mod pipeline;
pub mod render;
// 磁盘缓存与 `front`（前半链）是一体的：它的公开面里带着 `front::Rendered` /
// `front::RenderJob`，而那两个类型本就是 crate 内部的。整体压到 `pub(crate)` 比把
// `front` 的内部类型提升为对外 API 更诚实：这个模块没有对外契约。
pub(crate) mod rendercache;
pub mod run;
pub mod subset;
pub mod update;

use std::io::Write;

/// 退出码约定：0 成功；1 运行错误；2 用法错误。
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

pub fn dispatch(argv: Vec<String>) -> i32 {
    let inv = match args::parse(&argv) {
        Ok(i) => i,
        Err(e) => {
            if e.message == "help" {
                let mut out = std::io::stdout();
                let _ = out.write_all(args::USAGE.as_bytes());
                return EXIT_OK;
            }
            let mut err = std::io::stderr();
            let _ = writeln!(err, "{}", e.message);
            let _ = write!(err, "\n{}", args::USAGE);
            // 解析阶段失败一律 2（用法错）；1 留给"命令对了但跑失败"
            return EXIT_USAGE;
        }
    };
    let cmd = inv.command.name().to_string();
    // 22 §2：**一次执行一个 id**，在 dispatch 这一层生成。
    //
    // 为什么不在 `Envelope::ok/fail` 里各自生成：那样日志里记的那个 id
    // 与用户报告里看到的会是**两个不同的值** —— 而 correlation id 的全部用途
    // 就是让这两处能对上。"看起来像 UUID"不够，必须是同一个。
    let corr = core_model::CorrId::new_job();
    // 22 §1 / §2 的联动：把 id 放进 span，`execute` 里的每条 `tracing` event
    // 都自动带着它，不必在每处 `info!` 里重复写一遍。
    // 这也是选 `tracing` 而不是 `log` 的直接好处。
    let span = core_model::span_for(corr, "run");
    let _guard = span.enter();
    match run::execute(&inv) {
        Ok(env) => {
            // 批量路径会**自己**生成一个 id，并用它的子 id 去标识每个输出
            // （`output_ids` 的共享段就是它）。那种情况下信封上的 id 必须
            // 用**那一个**：否则报告里 `corr_id` 与信封上的 `corr_id` 是两个值，
            // 而 correlation id 的全部用途就是让这两处是同一个。
            //
            // 所以这里的规则是：命令自己带了就尊重它，没带才用本层的。
            let id = if env.came_with_corr_id {
                env.corr_id.clone()
            } else {
                corr.as_string()
            };
            emit(&env.with_corr_id(&id), inv.json)
        }
        Err(e) => emit(
            &output::Envelope::fail(&cmd, &e).with_corr_id(&corr.as_string()),
            inv.json,
        ),
    }
}

fn emit(env: &output::Envelope, json: bool) -> i32 {
    let mut out = std::io::stdout();
    let text = if json {
        format!("{}\n", env.to_json_line())
    } else {
        // 22 §2：人读输出末尾单独一行 corr-id，用户抄下来就能用。
        // 放在末尾而不是开头：失败时已有信息在前，一行 id 不抢眼但总在。
        format!("{}{}\n", env.to_text(), env.corr_line())
    };
    if out.write_all(text.as_bytes()).is_err() {
        // stdout 关闭（管道提前退出）不是任务失败，但必须能被诊断
        let mut err = std::io::stderr();
        let _ = writeln!(err, "STK-OUT-0001: 无法写出结果（stdout 已关闭）");
        return EXIT_ERROR;
    }
    let _ = out.flush();
    if env.ok {
        EXIT_OK
    } else {
        EXIT_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn help_exits_zero_and_usage_errors_exit_two() {
        assert_eq!(dispatch(argv(&["--help"])), EXIT_OK);
        assert_eq!(dispatch(argv(&[])), EXIT_USAGE);
        assert_eq!(dispatch(argv(&["nosuchcommand", "x"])), EXIT_USAGE);
    }

    #[test]
    fn missing_file_is_a_run_error_not_a_panic() {
        let code = dispatch(argv(&["probe", "Z:\\definitely-not-here-9f3a.ass"]));
        assert_eq!(code, EXIT_ERROR);
    }

    #[test]
    fn json_envelope_for_unbuilt_command() {
        let code = dispatch(argv(&["render", "x.ass"]));
        assert_eq!(code, EXIT_ERROR);
    }
}
