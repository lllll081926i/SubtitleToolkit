use std::io::Read;

fn main() -> std::process::ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // `@argsfile`：批量/CI 场景下命令行长度限制（20 §Repro 需要能重放长任务）
    let mut expanded: Vec<String> = Vec::with_capacity(argv.len());
    for a in argv.drain(..) {
        if let Some(path) = a.strip_prefix('@') {
            match read_argfile(path) {
                Ok(lines) => expanded.extend(lines),
                Err(e) => {
                    eprintln!("STK-INP-0140: 无法读取参数文件 {path}：{e}");
                    return std::process::ExitCode::from(2);
                }
            }
        } else {
            expanded.push(a);
        }
    }
    // 22 §1：日志在**解析之前**初始化，否则早期错误（例如参数拼错）就落不了盘。
    // 级别取自 36 §1 的 AppSettings；读设置本身不靠日志，所以没有循环依赖。
    // 注意 `STK_LOG` 优先于设置：与 `STK_DATA_DIR` 同一理由——
    // 排查“为什么日志没出来”时需要一条不依赖设置文件的路。
    let level = match std::env::var("STK_LOG") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            core_model::SettingsStore::current()
                .load()
                .settings
                .log_level
        }
    };
    core_model::init_logging(&level);
    tracing::debug!(log_level = %level, "日志已初始化");
    std::process::ExitCode::from(cli::dispatch(expanded) as u8)
}

fn read_argfile(path: &str) -> Result<Vec<String>, String> {
    let mut buf = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .read_to_string(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(' ') {
            Some((flag, value)) if flag.starts_with("--") => format!("{flag}={value}"),
            None => l.to_string(),
            Some(_) => l.to_string(),
        })
        .collect())
}
