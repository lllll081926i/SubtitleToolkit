//! 校验结果的统一模型（13 §6 报告要求：severity + segment offset + display set + PTS + 修复建议）。
//!
//! 这里刻意不使用 `StkError`：一次校验要给出**全部**发现（不是遇错即停），
//! 而且 warning 不该被调用方当成异常处理。`ValidationReport::to_error()` 才把
//! 达到 error/fatal 的结果折叠成错误，供流水线中断使用。

use core_model::profile::ValidationLevel;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Fatal => "fatal",
        }
    }
    /// error 及以上阻止 Strict badge（13 §6）。
    pub fn blocks_strict(self) -> bool {
        matches!(self, Severity::Error | Severity::Fatal)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// 检查点 id，形如 `PGS.STR.03`；稳定，UI/日志按它做本地化与去重。
    pub check: &'static str,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_set: Option<usize>,
    /// 90 kHz tick
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pts: Option<i64>,
    pub suggestion: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    pub fn new() -> ValidationReport {
        ValidationReport {
            findings: Vec::new(),
        }
    }

    pub fn add(
        &mut self,
        check: &'static str,
        severity: Severity,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> &mut Finding {
        self.findings.push(Finding {
            check,
            severity,
            message: message.into(),
            segment_offset: None,
            display_set: None,
            pts: None,
            suggestion: suggestion.into(),
        });
        self.findings.last_mut().expect("刚 push 过，必有最后一条")
    }

    pub fn worst(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).max()
    }

    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    pub fn has_check(&self, check: &str) -> bool {
        self.findings.iter().any(|f| f.check == check)
    }

    /// 没有任何 error/fatal。**这只说明"本次校验没发现问题"**，
    /// 不等于可以贴 Strict 徽章 —— 那要用 `strict_badge_allowed(level)`。
    pub fn no_blocking_findings(&self) -> bool {
        !self.findings.iter().any(|f| f.severity.blocks_strict())
    }

    /// 本次校验有没有落在"未实现层"上。这些层不报 error（否则 strict 就没法用了），
    /// 但它们的存在足以让 Strict 徽章作废：没跑过的层不能被当成"通过"。
    ///
    /// **2026-10-17 变更**：第 5 层（解码器模型）已实现，所以 `PGS.DEC.01`
    /// **不再**是覆盖缺口——它现在是一个真不变式（"段 PTS 晚于本集 PCS"）。
    /// 把已实现的层继续列在这里会让一条真实的时序警告被读成"层没做"，
    /// 而徽章却被扣在别的原因上。缺口清单只能列真的没跑的层。
    ///
    /// 层 5 里确实没做的部分（缓冲区占用曲线、acquisition 分支）用 `PGS.DEC.20`
    /// 在报告里说明，但它**不是**徽章门禁：13 §6 要求的是"每层附未覆盖范围"，
    /// 而那一条的缺失已由 `PGS.DEV.01`（整个第 7 层未做）代表。
    pub const COVERAGE_GAP_CHECKS: [&'static str; 1] = ["PGS.DEV.01"];

    /// 本次报告是否声明了某个未覆盖的校验层。
    pub fn has_coverage_gap(&self) -> bool {
        self.findings
            .iter()
            .any(|f| Self::COVERAGE_GAP_CHECKS.contains(&f.check))
    }

    /// Strict 徽章的三个必要条件（13 §6）：
    ///
    /// 1. 这次确实**按 Strict** 跑了七层里我们能跑的部分（Basic/Standard 的报告不能倒推 Strict）；
    /// 2. 没有 error/fatal；
    /// 3. 没有落在未实现层上的覆盖缺口。
    ///
    /// 当前仍覆盖不到第 7 层（设备语料），按 Strict 跑时一定会拿到 `PGS.DEV.01`，
    /// 因此**仍总是返回 false**——但它是 warning，不会阻断 strict 对其余规则的检查。
    /// 第 5 层已实现，不再计入缺口。
    pub fn strict_badge_allowed(&self, validated_at: ValidationLevel) -> bool {
        validated_at == ValidationLevel::Strict
            && self.no_blocking_findings()
            && !self.has_coverage_gap()
    }

    pub fn merge(&mut self, other: ValidationReport) {
        self.findings.extend(other.findings);
    }

    pub fn summary(&self) -> String {
        format!(
            "info={} warning={} error={} fatal={}",
            self.count(Severity::Info),
            self.count(Severity::Warning),
            self.count(Severity::Error),
            self.count(Severity::Fatal)
        )
    }

    pub fn text_lines(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(|f| {
                let mut s = format!("[{}] {}: {}", f.severity.label(), f.check, f.message);
                if let Some(o) = f.segment_offset {
                    s.push_str(&format!("｜段偏移 {o}"));
                }
                if let Some(d) = f.display_set {
                    s.push_str(&format!("｜display set {d}"));
                }
                if let Some(p) = f.pts {
                    s.push_str(&format!("｜PTS {p}"));
                }
                s.push_str(&format!("｜建议：{}", f.suggestion));
                s
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_badge_requires_strict_level_and_no_error_or_fatal() {
        use core_model::profile::ValidationLevel;
        let mut r = ValidationReport::new();
        assert!(r.no_blocking_findings(), "空报告没有阻断项");
        // 但"没跑过 Strict"就不可能给出 Strict 徽章
        assert!(
            !r.strict_badge_allowed(ValidationLevel::Basic),
            "Basic 报告不能倒推 Strict"
        );
        assert!(!r.strict_badge_allowed(ValidationLevel::Standard));
        assert!(r.strict_badge_allowed(ValidationLevel::Strict));
        r.add("PGS.PRO.01", Severity::Info, "画布 1920x1080", "无需处理");
        assert!(r.no_blocking_findings());
        r.add(
            "PGS.PRO.02",
            Severity::Warning,
            "2160p 画布",
            "只作非标准输出",
        );
        assert!(r.no_blocking_findings(), "warning 只降级，不否决");
        let f = r.add("PGS.TML.01", Severity::Error, "DTS 晚于 PTS", "重新 author");
        f.pts = Some(90_000);
        f.display_set = Some(3);
        f.segment_offset = Some(41);
        assert!(!r.no_blocking_findings());
        assert!(!r.strict_badge_allowed(ValidationLevel::Strict));
        assert_eq!(r.worst(), Some(Severity::Error));
        let line = r.text_lines().pop().unwrap();
        assert!(
            line.contains("PGS.TML.01") && line.contains("段偏移 41") && line.contains("PTS 90000"),
            "{line}"
        );
    }

    #[test]
    fn merge_keeps_every_finding() {
        let mut a = ValidationReport::new();
        a.add("X.01", Severity::Warning, "a", "x");
        let mut b = ValidationReport::new();
        b.add("X.02", Severity::Fatal, "b", "y");
        a.merge(b);
        assert_eq!(a.findings.len(), 2);
        assert_eq!(a.count(Severity::Fatal), 1);
        assert!(a.summary().contains("fatal=1"));
        assert!(a.has_check("X.01") && !a.has_check("X.03"));
    }
}
