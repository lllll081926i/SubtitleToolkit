//! 任务抽屉的状态（15 §任务凭据、38 §2 判定）。
//!
//! 状态字符串与 CLI 报告里的 `status` **逐字一致**（`verified` / `from-cache` /
//! `would-write` / `failed` / `cancelled`）。GUI 不另造一套词：
//! 同一件事在两个地方叫两个名字，最后没人能对齐。
//!
//! ## 为什么有一条 `Skipped` 之外的 `NotAttempted`
//!
//! 计划的每一格（输入 × profile）在开始时就先摆进列表，状态是 `Queued`。
//! 任务因为 spec 起不来而根本没跑时，它们必须从 `Queued` 变成 `NotAttempted`
//! ——留在 `Queued` 会让用户以为"还在排队"，而其实已经结束了。

use std::path::PathBuf;

/// 一格任务的状态。
#[derive(Clone, Debug, PartialEq)]
pub enum TaskStatus {
    /// 已计划，还没跑
    Queued,
    /// 正在跑（CLI 报告是逐条出来的，所以这里是粗粒度）
    Running,
    /// CLI 报告里的 `verified`
    Verified {
        detail: String,
    },
    /// CLI 报告里的 `from-cache`（含 overwrite=skip 命中）
    FromCache {
        detail: String,
    },
    /// CLI 报告里的 `would-write`（dry-run）
    WouldWrite {
        detail: String,
    },
    /// CLI 报告里的 `failed` / `cancelled`
    Failed {
        code: String,
        detail: String,
    },
    Cancelled {
        detail: String,
    },
    /// 任务根本没跑到这一格（spec 起不来、被取消、引擎拒绝）
    NotAttempted {
        reason: String,
    },
}

impl TaskStatus {
    /// 从 CLI 报告里的 status 字符串翻译过来。
    ///
    /// 认不出的字符串**不当成功**：它会落成 `Failed` 并带上原文。
    /// 把一个未知状态当成功，正是"看起来跑完了其实没跑"的入口。
    pub fn from_report(status: &str, detail: &str) -> TaskStatus {
        match status {
            "verified" => TaskStatus::Verified {
                detail: detail.to_string(),
            },
            "from-cache" => TaskStatus::FromCache {
                detail: detail.to_string(),
            },
            "would-write" => TaskStatus::WouldWrite {
                detail: detail.to_string(),
            },
            "cancelled" => TaskStatus::Cancelled {
                detail: detail.to_string(),
            },
            "failed" => TaskStatus::Failed {
                code: extract_code(detail),
                detail: detail.to_string(),
            },
            other => TaskStatus::Failed {
                code: format!("unknown:{other}"),
                detail: format!("报告里的状态无法识别：{other}；{detail}"),
            },
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self,
            TaskStatus::Verified { .. }
                | TaskStatus::FromCache { .. }
                | TaskStatus::WouldWrite { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            TaskStatus::Queued => "排队中",
            TaskStatus::Running => "执行中",
            TaskStatus::Verified { .. } => "已完成",
            TaskStatus::FromCache { .. } => "已复用",
            TaskStatus::WouldWrite { .. } => "演练",
            TaskStatus::Failed { .. } => "失败",
            TaskStatus::Cancelled { .. } => "已取消",
            TaskStatus::NotAttempted { .. } => "未执行",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            TaskStatus::Verified { detail }
            | TaskStatus::FromCache { detail }
            | TaskStatus::WouldWrite { detail }
            | TaskStatus::Cancelled { detail } => detail,
            TaskStatus::Failed { detail, .. } => detail,
            TaskStatus::NotAttempted { reason } => reason,
            TaskStatus::Queued | TaskStatus::Running => "",
        }
    }
}

/// 从 CLI 的 detail 文本里挑出错误码（形如 `STK-INP-0110: ...`）。
///
/// 挑不出来就**如实说不知道**，不编一个码：编出来的码会让人去查一个不存在的东西。
fn extract_code(detail: &str) -> String {
    let head = detail.split(':').next().unwrap_or("").trim();
    if head.starts_with("STK-") {
        return head.to_string();
    }
    // `stage: STK-XXX-0000: message` 这种形状
    for part in detail.split_whitespace() {
        let p = part.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        if p.starts_with("STK-") && p.matches('-').count() >= 2 {
            return p.to_string();
        }
    }
    "未提供错误码".to_string()
}

#[derive(Clone, Debug)]
pub struct TaskItem {
    pub id: usize,
    /// 输入的完整路径（报告里就是它；只留文件名会让两个同名文件分不清）
    pub input: String,
    pub input_name: String,
    pub profile_name: String,
    pub status: TaskStatus,
}

#[derive(Clone, Debug)]
pub struct TaskDrawerState {
    pub is_expanded: bool,
    pub output_dir: PathBuf,
    pub tasks: Vec<TaskItem>,
    pub is_running: bool,
    /// 编译期的"做不到"说明 + 报告里的 warnings
    pub notes: Vec<String>,
    /// 最近一次任务的整体判定（success / partial-success / failed / cancelled）
    pub outcome: Option<String>,
    /// spec 落在哪：用户想拿去 CLI 重放时要知道东西在哪
    pub spec_path: Option<PathBuf>,
    /// 报告落在哪
    pub report_path: Option<PathBuf>,
}

impl Default for TaskDrawerState {
    fn default() -> Self {
        let default_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("outputs");
        Self {
            is_expanded: false,
            output_dir: default_dir,
            tasks: Vec::new(),
            is_running: false,
            notes: Vec::new(),
            outcome: None,
            spec_path: None,
            report_path: None,
        }
    }
}

impl TaskDrawerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_expand(&mut self) {
        self.is_expanded = !self.is_expanded;
    }

    /// 按 (输入, profile) 摆出计划。重复调用会重置上一轮的结果。
    pub fn plan(&mut self, inputs: &[(String, String)], profiles: &[String]) {
        self.tasks.clear();
        self.notes.clear();
        self.outcome = None;
        self.spec_path = None;
        self.report_path = None;
        let mut id = 1;
        for (path, name) in inputs {
            for p in profiles {
                self.tasks.push(TaskItem {
                    id,
                    input: path.clone(),
                    input_name: name.clone(),
                    profile_name: p.clone(),
                    status: TaskStatus::Queued,
                });
                id += 1;
            }
        }
    }

    /// 把一行的最新状态写进去。找不到对应行就**新增一格**：
    /// 报告里出现了我们没计划过的组合，那是必须看得见的异常，不是可以丢掉的噪声。
    pub fn apply(&mut self, input: &str, profile: &str, status: TaskStatus) {
        if let Some(t) = self
            .tasks
            .iter_mut()
            .find(|t| t.input == input && t.profile_name == profile)
        {
            t.status = status;
            return;
        }
        self.tasks.push(TaskItem {
            id: self.tasks.len() + 1,
            input: input.to_string(),
            input_name: short_name(input),
            profile_name: profile.to_string(),
            status,
        });
        self.notes.push(format!(
            "报告里出现了计划外的组合（{profile} × {}），已单独列出",
            short_name(input)
        ));
    }

    /// 任务整体结束：还停在 Queued/Running 的格子必须落成"未执行"。
    pub fn finish(&mut self, outcome: &str, reason: &str) {
        for t in self.tasks.iter_mut() {
            if matches!(t.status, TaskStatus::Queued | TaskStatus::Running) {
                t.status = TaskStatus::NotAttempted {
                    reason: reason.to_string(),
                };
            }
        }
        self.is_running = false;
        self.outcome = Some(outcome.to_string());
    }

    pub fn total_tasks(&self) -> usize {
        self.tasks.len()
    }

    pub fn completed_tasks(&self) -> usize {
        self.tasks.iter().filter(|t| t.status.is_success()).count()
    }

    pub fn failed_tasks(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .count()
    }

    pub fn not_attempted_tasks(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::NotAttempted { .. }))
            .count()
    }

    /// 整体判定：只要有任何一格没成功或没跑，就不是 success。
    /// 与 CLI 的 `JobOutcome` 同一套优先级（14 §Multi-output）。
    pub fn derived_outcome(&self) -> &'static str {
        let total = self.tasks.len();
        if total == 0 {
            return "empty";
        }
        let good = self.completed_tasks();
        if good == total {
            "success"
        } else if good == 0 {
            if self
                .tasks
                .iter()
                .all(|t| matches!(t.status, TaskStatus::Cancelled { .. }))
            {
                "cancelled"
            } else {
                "failed"
            }
        } else {
            "partial-success"
        }
    }
}

fn short_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_status_strings_map_without_inventing_synonyms() {
        assert_eq!(
            TaskStatus::from_report("verified", "p sha256=ab"),
            TaskStatus::Verified {
                detail: "p sha256=ab".into()
            }
        );
        assert_eq!(
            TaskStatus::from_report("from-cache", "p"),
            TaskStatus::FromCache { detail: "p".into() }
        );
        assert_eq!(
            TaskStatus::from_report("would-write", "p"),
            TaskStatus::WouldWrite { detail: "p".into() }
        );
        assert!(matches!(
            TaskStatus::from_report("cancelled", ""),
            TaskStatus::Cancelled { .. }
        ));
    }

    /// 认不出的状态**绝不能**当成功：那是"看起来跑完了其实没跑"的入口。
    #[test]
    fn an_unknown_status_is_a_failure_not_a_success() {
        let s = TaskStatus::from_report("all-good-i-promise", "trust me");
        assert!(!s.is_success(), "{s:?}");
        match s {
            TaskStatus::Failed { code, detail } => {
                assert!(code.contains("all-good-i-promise"), "{code}");
                assert!(detail.contains("无法识别"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn failure_detail_yields_a_real_error_code_or_says_it_has_none() {
        assert_eq!(
            extract_code("STK-INP-0110: 无法读取输入文件"),
            "STK-INP-0110"
        );
        assert_eq!(
            extract_code("pgs encode: STK-PGS-0040: 超色"),
            "STK-PGS-0040"
        );
        // 没有码就如实说"未提供"，不编一个让人去查不存在的码
        assert_eq!(extract_code("something went wrong"), "未提供错误码");
    }

    #[test]
    fn plan_creates_one_row_per_input_times_profile() {
        let mut d = TaskDrawerState::new();
        d.plan(
            &[
                (r"D:\in\a.ass".into(), "a.ass".into()),
                (r"D:\in\b.ass".into(), "b.ass".into()),
            ],
            &["P1".into(), "P2".into()],
        );
        assert_eq!(d.total_tasks(), 4);
        assert_eq!(d.completed_tasks(), 0);
        assert!(d.tasks.iter().all(|t| t.status == TaskStatus::Queued));
    }

    /// 任务根本没跑时，格子不能永远停在"排队中"——用户会以为还在等。
    #[test]
    fn finish_turns_untouched_rows_into_not_attempted() {
        let mut d = TaskDrawerState::new();
        d.plan(
            &[(r"D:\in\a.ass".into(), "a.ass".into())],
            &["P1".into(), "P2".into()],
        );
        d.apply(
            r"D:\in\a.ass",
            "P1",
            TaskStatus::Verified {
                detail: "ok".into(),
            },
        );
        d.finish("failed", "batch spec 起不来");
        assert_eq!(d.completed_tasks(), 1);
        assert_eq!(d.not_attempted_tasks(), 1, "没跑到的那一格要标明");
        assert_eq!(d.derived_outcome(), "partial-success");
        assert!(!d.is_running);
    }

    #[test]
    fn derived_outcome_matches_the_cli_priority() {
        let mut d = TaskDrawerState::new();
        assert_eq!(d.derived_outcome(), "empty");
        d.plan(&[("a".into(), "a".into())], &["P".into()]);
        assert_eq!(d.derived_outcome(), "failed", "全 Queued = 一个都没成");
        d.apply(
            "a",
            "P",
            TaskStatus::Verified {
                detail: String::new(),
            },
        );
        assert_eq!(d.derived_outcome(), "success");
        d.plan(&[("a".into(), "a".into())], &["P".into()]);
        d.apply(
            "a",
            "P",
            TaskStatus::Failed {
                code: "STK-X".into(),
                detail: "no".into(),
            },
        );
        assert_eq!(d.derived_outcome(), "failed");
    }

    /// 报告里出现的计划外组合必须看得见，不能被丢掉。
    #[test]
    fn an_unplanned_row_is_added_and_noted() {
        let mut d = TaskDrawerState::new();
        d.plan(&[("a".into(), "a".into())], &["P".into()]);
        d.apply(
            r"D:\other\z.ass",
            "Q",
            TaskStatus::Verified {
                detail: "ok".into(),
            },
        );
        assert_eq!(d.total_tasks(), 2);
        assert!(
            d.notes.iter().any(|n| n.contains("计划外")),
            "计划外的行必须留下说明：{:?}",
            d.notes
        );
    }

    #[test]
    fn from_cache_counts_as_success_because_skip_is_a_success() {
        // 15 §6 的 skip 语义就是"别动它"，那是成功而不是失败
        assert!(TaskStatus::FromCache {
            detail: String::new()
        }
        .is_success());
        assert!(TaskStatus::WouldWrite {
            detail: String::new()
        }
        .is_success());
        assert!(!TaskStatus::NotAttempted {
            reason: String::new()
        }
        .is_success());
    }
}
