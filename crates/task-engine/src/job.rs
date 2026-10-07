//! Job 模型、partial success 分类、崩溃重启预算与协作取消（15 §1/§8、38 §2/§3/§7）。
//!
//! 这一层刻意**不执行**任何东西：它只规定"任务能处在什么状态、失败怎么分类、
//! 什么时候必须停手"。真正的执行器（渲染/author/写盘）在各自 crate 里，
//! 由 GUI/CLI 组装。把状态机单独拿出来，是因为 38 §2 的 `PartialSuccess`
//! 最容易被实现成"有失败就整体失败"或"有成功就整体成功"，两种都骗人。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::profile::FormatKind;
use core_model::time::Fps;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputProfile {
    /// 用户可见的名字，例如 `1080p-ASS` / `BDN`
    pub name: String,
    pub format: FormatKind,
    pub canvas: Canvas,
    pub fps: Fps,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSet {
    pub files: Vec<String>,
    /// 主输入的 SHA-256（15 §4 缓存 key 的第一分量，也是 01 §"输入摘要"的凭据）
    pub primary_sha256: String,
}

impl InputSet {
    pub fn validate(&self) -> Result<()> {
        if self.files.is_empty() {
            return Err(StkError::simple(Domain::Input, 80, "输入集合是空的"));
        }
        if self.primary_sha256.len() != 64
            || !self.primary_sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(
                StkError::simple(Domain::Input, 81, "主输入摘要缺失或不是 sha256 hex")
                    .with_detail("任务必须能回答「这次跑的到底是哪份输入」（01 §任务凭据）"),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub input: InputSet,
    pub profiles: Vec<OutputProfile>,
}

impl Job {
    pub fn validate(&self) -> Result<()> {
        self.input.validate()?;
        if self.profiles.is_empty() {
            return Err(StkError::simple(
                Domain::Input,
                82,
                "任务没有任何输出 profile",
            ));
        }
        let mut names: Vec<&str> = self.profiles.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err(StkError::simple(Domain::Input, 83, "输出 profile 名字重复")
                .with_action("同名的两个 profile 会让报告与恢复状态分不清谁是谁"));
        }
        Ok(())
    }

    /// 15 §3：renderer 按 provider 设 semaphore。profile 数 × 每个 provider 的额度
    /// 不在这里决定，但至少要能回答"这个任务需要几个并发槽位"。
    pub fn wants_concurrency(&self) -> usize {
        self.profiles.len()
    }
}

/// 单个 profile 的最终状态。`Verified` 的含义是**已经按 38 §1 的路径写完并回读校验过**，
/// 不是"函数返回了"。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileOutcome {
    /// dry-run：产物被完整生成并校验过，但**没有落盘**，所以拿不到可用哈希。
    /// 单独立一个变体而不是借用 Verified：`is_success()` 不包含它，
    /// 报告里也不会出现"verified 但其实文件不存在"这种话。
    WouldWrite {
        path: String,
        bytes: u64,
    },
    Verified {
        path: String,
        sha256: String,
    },
    FromCache {
        path: String,
        sha256: String,
    },
    Failed {
        stage: String,
        message: String,
    },
    Cancelled {
        stage: String,
    },
}

impl ProfileOutcome {
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            ProfileOutcome::Verified { .. } | ProfileOutcome::FromCache { .. }
        )
    }
}

/// 任务级分类（38 §2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobOutcome {
    Success,
    PartialSuccess,
    Failed,
    Cancelled,
}

impl JobOutcome {
    pub fn label(self) -> &'static str {
        match self {
            JobOutcome::Success => "success",
            JobOutcome::PartialSuccess => "partial-success",
            JobOutcome::Failed => "failed",
            JobOutcome::Cancelled => "cancelled",
        }
    }

    /// 分类规则：只要有任何 profile 被取消，整个任务算 Cancelled（用户主动停手优先于统计）；
    /// 全成功才 Success；一个都没成才是 Failed；其余是 PartialSuccess。
    pub fn classify(outcomes: &[ProfileOutcome]) -> JobOutcome {
        if outcomes
            .iter()
            .any(|o| matches!(o, ProfileOutcome::Cancelled { .. }))
        {
            return JobOutcome::Cancelled;
        }
        let ok = outcomes.iter().filter(|o| o.is_success()).count();
        if ok == outcomes.len() && !outcomes.is_empty() {
            JobOutcome::Success
        } else if ok == 0 {
            JobOutcome::Failed
        } else {
            JobOutcome::PartialSuccess
        }
    }
}

/// 15 §8 的批量摘要（JSON）。字段刻意包含 warning 与耗时，而不只是"成功/失败"。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReport {
    pub job_id: String,
    pub outcome: String,
    pub profiles: Vec<ProfileReport>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
    pub renderer: Option<String>,
    pub tool_versions: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileReport {
    pub name: String,
    pub status: String,
    pub detail: String,
}

impl JobReport {
    pub fn build(
        job: &Job,
        outcomes: &[ProfileOutcome],
        warnings: Vec<String>,
        elapsed_ms: u64,
    ) -> Result<JobReport> {
        if job.profiles.len() != outcomes.len() {
            return Err(
                StkError::simple(Domain::Output, 30, "报告长度与 profile 数不一致").with_detail(
                    format!(
                        "profiles={} outcomes={}",
                        job.profiles.len(),
                        outcomes.len()
                    ),
                ),
            );
        }
        let outcome = JobOutcome::classify(outcomes);
        let profiles = job
            .profiles
            .iter()
            .zip(outcomes)
            .map(|(p, o)| ProfileReport {
                name: p.name.clone(),
                status: match o {
                    ProfileOutcome::WouldWrite { .. } => "would-write",
                    ProfileOutcome::Verified { .. } => "verified",
                    ProfileOutcome::FromCache { .. } => "from-cache",
                    ProfileOutcome::Failed { .. } => "failed",
                    ProfileOutcome::Cancelled { .. } => "cancelled",
                }
                .to_string(),
                detail: match o {
                    ProfileOutcome::WouldWrite { path, bytes } => {
                        format!("{path}（dry-run：{bytes} 字节未写盘）")
                    }
                    ProfileOutcome::Verified { path, .. }
                    | ProfileOutcome::FromCache { path, .. } => path.clone(),
                    ProfileOutcome::Failed { stage, message } => format!("{stage}: {message}"),
                    ProfileOutcome::Cancelled { stage } => stage.clone(),
                },
            })
            .collect();
        Ok(JobReport {
            job_id: job.id.clone(),
            outcome: outcome.label().to_string(),
            profiles,
            warnings,
            elapsed_ms,
            renderer: None,
            tool_versions: Vec::new(),
        })
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| {
            StkError::simple(Domain::Output, 31, "任务摘要序列化失败").with_detail(e.to_string())
        })
    }
}

/// 38 §3：renderer 崩溃最多自动重启一次，第二次就停掉**这个输出**，
/// 其它不依赖 renderer 的输出继续。预算是 per-profile 的，不是全局的。
#[derive(Clone, Copy, Debug)]
pub struct CrashBudget {
    pub remaining: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashAction {
    /// 重启 host 后重试当前 stage
    RestartOnce,
    /// 停止该 output（不循环重启）
    StopProfile,
}

impl CrashBudget {
    /// 38 §3 的"仅无用户数据风险时"：默认给一次预算，调用方必须显式声明是否安全。
    pub fn new(safe_to_restart: bool) -> CrashBudget {
        CrashBudget {
            remaining: u8::from(safe_to_restart),
        }
    }

    pub fn on_crash(&mut self) -> CrashAction {
        if self.remaining > 0 {
            self.remaining -= 1;
            CrashAction::RestartOnce
        } else {
            CrashAction::StopProfile
        }
    }
}

/// 38 §7：协作式取消。writer 在 atomic unit 完成后检查，不在中途打断写盘。
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// 在 atomic unit 边界调用；返回 Err 表示"这一单元做完就停"。
    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            return Err(StkError::simple(Domain::Output, 32, "任务已取消")
                .with_action("已完成的原子写入会保留，不会留下半成品"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::profile::FormatKind;

    fn profile(name: &str, format: FormatKind) -> OutputProfile {
        OutputProfile {
            name: name.into(),
            format,
            canvas: Canvas::FHD1080,
            fps: Fps::PAL_25,
        }
    }

    fn job() -> Job {
        Job {
            id: "j1".into(),
            input: InputSet {
                files: vec!["a.ass".into()],
                primary_sha256: "a".repeat(64),
            },
            profiles: vec![
                profile("1080p-ASS", FormatKind::Ass),
                profile("BDN", FormatKind::Bdn),
                profile("SUP", FormatKind::Pgs),
            ],
        }
    }

    #[test]
    fn partial_success_keeps_verified_outputs_and_lists_failures() {
        // 38 §2 的字面例子：1080 ASS 成功、SUP 失败 -> PartialSuccess
        let outcomes = vec![
            ProfileOutcome::Verified {
                path: "a.ass".into(),
                sha256: "b".repeat(64),
            },
            ProfileOutcome::Verified {
                path: "a.zip".into(),
                sha256: "c".repeat(64),
            },
            ProfileOutcome::Failed {
                stage: "author".into(),
                message: "色彩数超过 PGS 调色板上限".into(),
            },
        ];
        assert_eq!(
            JobOutcome::classify(&outcomes),
            JobOutcome::PartialSuccess,
            "有成功也有失败绝不能报成 success 或 failed"
        );
        let rep = JobReport::build(&job(), &outcomes, vec!["warning!".into()], 1200).unwrap();
        assert_eq!(rep.outcome, "partial-success");
        assert_eq!(rep.profiles[2].status, "failed");
        assert!(rep.profiles[2].detail.contains("author"));
        assert_eq!(rep.warnings.len(), 1);
        let json = rep.to_json().unwrap();
        assert!(json.contains("\"partial-success\""), "{json}");
        assert!(!json.contains('\n'), "摘要必须是单行 JSON");
    }

    #[test]
    fn all_verified_is_success_and_none_verified_is_failed() {
        let all_ok = vec![
            ProfileOutcome::Verified {
                path: "x".into(),
                sha256: "a".repeat(64),
            },
            ProfileOutcome::FromCache {
                path: "y".into(),
                sha256: "b".repeat(64),
            },
        ];
        assert_eq!(JobOutcome::classify(&all_ok), JobOutcome::Success);
        let none = vec![ProfileOutcome::Failed {
            stage: "render".into(),
            message: "host 退出".into(),
        }];
        assert_eq!(JobOutcome::classify(&none), JobOutcome::Failed);
        assert_eq!(
            JobOutcome::classify(&[]),
            JobOutcome::Failed,
            "空结果不是成功"
        );
    }

    #[test]
    fn cancel_beats_the_statistics() {
        let mixed = vec![
            ProfileOutcome::Verified {
                path: "x".into(),
                sha256: "a".repeat(64),
            },
            ProfileOutcome::Cancelled {
                stage: "write".into(),
            },
        ];
        assert_eq!(JobOutcome::classify(&mixed), JobOutcome::Cancelled);
    }

    #[test]
    fn crash_budget_restarts_once_then_stops_only_that_profile() {
        // 38 §3
        let mut safe = CrashBudget::new(true);
        assert_eq!(safe.on_crash(), CrashAction::RestartOnce);
        assert_eq!(safe.on_crash(), CrashAction::StopProfile, "不允许循环重启");
        assert_eq!(safe.on_crash(), CrashAction::StopProfile);
        let mut unsafe_budget = CrashBudget::new(false);
        assert_eq!(
            unsafe_budget.on_crash(),
            CrashAction::StopProfile,
            "有用户数据风险时一次都不重启"
        );
    }

    #[test]
    fn cancel_token_is_cooperative_and_shared() {
        let t = CancelToken::new();
        let clone = t.clone();
        assert!(!clone.cancelled());
        t.cancel();
        assert!(clone.cancelled(), "取消信号必须跨克隆可见");
        let e = clone.check().unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0032");
    }

    #[test]
    fn job_validation_catches_ambiguous_profiles() {
        assert!(job().validate().is_ok());
        let mut dup = job();
        dup.profiles[1].name = "1080p-ASS".into();
        let e = dup.validate().unwrap_err();
        assert_eq!(e.code_string(), "STK-INP-0083");

        let mut nohash = job();
        nohash.input.primary_sha256 = String::new();
        assert_eq!(
            nohash.validate().unwrap_err().code_string(),
            "STK-INP-0081",
            "没有输入摘要的任务不可追溯（01 §每个任务都有输入摘要）"
        );

        let mut empty = job();
        empty.profiles.clear();
        assert!(empty.validate().is_err());
    }

    #[test]
    fn report_refuses_length_mismatch() {
        let e = JobReport::build(
            &job(),
            &[ProfileOutcome::Failed {
                stage: "s".into(),
                message: "m".into(),
            }],
            Vec::new(),
            0,
        )
        .unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0030");
    }
}
