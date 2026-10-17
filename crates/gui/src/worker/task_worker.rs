use crate::state::profile::OutputProfile;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 异步批处理任务执行器
pub async fn execute_batch(
    inputs: Vec<PathBuf>,
    profiles: Vec<OutputProfile>,
    _output_dir: PathBuf,
    cancel_token: Arc<AtomicBool>,
    progress_tx: tokio::sync::mpsc::UnboundedSender<(usize, f32, String)>,
) -> Vec<(usize, Result<(), (String, String, String)>)> {
    tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        let total = inputs.len() * profiles.len();
        let mut current = 0;

        for input in &inputs {
            for profile in &profiles {
                if cancel_token.load(Ordering::Relaxed) {
                    results.push((
                        current,
                        Err((
                            "STK-CAN-0001".into(),
                            "用户取消".into(),
                            "任务被主动中止".into(),
                        )),
                    ));
                    current += 1;
                    continue;
                }

                current += 1;
                let pct = current as f32 / total as f32;
                let _ = progress_tx.send((
                    current,
                    pct,
                    format!(
                        "正在导出: {} -> {}",
                        input.file_name().unwrap().to_string_lossy(),
                        profile.name
                    ),
                ));

                // 模拟处理或调用 task-engine 输出事务
                std::thread::sleep(std::time::Duration::from_millis(50));

                results.push((current, Ok(())));
            }
        }
        results
    })
    .await
    .unwrap_or_default()
}
