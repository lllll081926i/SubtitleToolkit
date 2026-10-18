//! 统一错误模型（22_LOGGING_DIAGNOSTICS.md）：`STK-<域>-<编号>` + 用户可执行动作 + 技术细节。

use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Domain {
    Input,
    Parse,
    Transform,
    Render,
    Filter,
    Image,
    Pgs,
    Bdn,
    Container,
    Output,
    Validate,
    Dependency,
    /// 37 §2 的磁盘渲染缓存。
    ///
    /// 单独立一域而不是并进 `Output`：缓存的故障语义与输出不同 ——
    /// 输出失败意味着用户拿不到东西，缓存失败只意味着这一次慢一点。
    /// 混在一个域里，读报告的人分不清「产物坏了」和「缓存坏了」。
    Cache,
    /// 36 §1 第三层 Preset。
    ///
    /// 单独立一域：preset 的故障（读不开、schema 太新、字段非法）
    /// 与“任务自身的输入不对”是两件事。混在 `Input` 里，
    /// 用户分不清“我的字幕有问题”还是“我的配置有问题”。
    Preset,
}

impl Domain {
    pub fn tag(self) -> &'static str {
        match self {
            Domain::Input => "INP",
            Domain::Parse => "PRS",
            Domain::Transform => "TRF",
            Domain::Render => "RND",
            Domain::Filter => "FLT",
            Domain::Image => "IMG",
            Domain::Pgs => "PGS",
            Domain::Bdn => "BDN",
            Domain::Container => "CTR",
            Domain::Output => "OUT",
            Domain::Validate => "VAL",
            Domain::Dependency => "DEP",
            Domain::Cache => "RCA",
            Domain::Preset => "PRE",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StkError {
    pub domain: Domain,
    pub code: u16,
    pub title: String,
    pub user_action: Option<String>,
    pub details: Vec<String>,
}

impl StkError {
    pub fn simple(domain: Domain, code: u16, title: &str) -> StkError {
        StkError {
            domain,
            code,
            title: title.to_string(),
            user_action: None,
            details: Vec::new(),
        }
    }

    pub fn with_action(mut self, action: &str) -> StkError {
        self.user_action = Some(action.to_string());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> StkError {
        self.details.push(detail.into());
        self
    }

    /// `STK-PGS-0104` 形式。
    pub fn code_string(&self) -> String {
        format!("STK-{}-{:04}", self.domain.tag(), self.code)
    }
}

impl fmt::Display for StkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code_string(), self.title)?;
        if let Some(a) = &self.user_action {
            write!(f, "｜建议操作：{a}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StkError {}

impl From<std::io::Error> for StkError {
    fn from(e: std::io::Error) -> StkError {
        StkError::simple(Domain::Input, 1, "IO 错误").with_detail(e.to_string())
    }
}

pub type Result<T> = core::result::Result<T, StkError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_format_matches_spec() {
        let e = StkError::simple(Domain::Pgs, 104, "ODS 长度字段与实际 RLE 不一致")
            .with_action("重新编码该显示集");
        assert_eq!(e.code_string(), "STK-PGS-0104");
        assert!(e.to_string().contains("STK-PGS-0104"));
        assert!(e.to_string().contains("重新编码该显示集"));
    }

    #[test]
    fn every_domain_has_a_tag() {
        let all = [
            Domain::Input,
            Domain::Parse,
            Domain::Transform,
            Domain::Render,
            Domain::Filter,
            Domain::Image,
            Domain::Pgs,
            Domain::Bdn,
            Domain::Container,
            Domain::Output,
            Domain::Validate,
            Domain::Dependency,
            Domain::Cache,
        ];
        assert_eq!(all.len(), 13);
        for d in all {
            assert_eq!(d.tag().len(), 3);
        }
    }
}
