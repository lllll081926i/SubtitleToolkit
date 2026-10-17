//! subtitletool-cli 入口（20_CLI_AUTOMATION.md）。
//!
//! stdout 只放结果（`--json` 时是一行 JSONL 信封，否则是人读摘要）；
//! 进度与诊断一律走 stderr，这样 `> out.json` 不会被污染。

pub mod args;
pub mod batch;
pub mod deliver;
pub mod font;
pub(crate) mod fontenv;
pub(crate) mod front;
pub mod mkv;
pub mod output;
pub mod pipeline;
pub mod render;
pub mod run;
pub mod subset;

use core_model::error::StkError;
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
    match run::execute(&inv) {
        Ok(env) => emit(&env, inv.json),
        Err(e) => emit(&EnvelopeFailed(&e).build(&cmd), inv.json),
    }
}

struct EnvelopeFailed<'a>(&'a StkError);

impl EnvelopeFailed<'_> {
    fn build(&self, cmd: &str) -> output::Envelope {
        output::Envelope::fail(cmd, self.0)
    }
}

fn emit(env: &output::Envelope, json: bool) -> i32 {
    let mut out = std::io::stdout();
    let text = if json {
        format!("{}\n", env.to_json_line())
    } else {
        env.to_text()
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
