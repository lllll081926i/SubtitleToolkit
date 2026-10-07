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
