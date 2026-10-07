//! CLI 输出信封（20 §JSON：稳定 schema；进度走 stderr，绝不污染 stdout 结果）。
//!
//! schema 版本只增不减；字段一旦发布就不再改语义，新增字段用 `skip_serializing_if`
//! 保持老消费者的解析兼容。

use core_model::error::StkError;
use serde::Serialize;

pub const SCHEMA: u32 = 1;
pub const TOOL: &str = "subtitletool-cli";

#[derive(Clone, Debug, Serialize)]
pub struct ErrorEntry {
    pub code: String,
    pub domain: String,
    pub title: String,
    pub details: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl ErrorEntry {
    pub fn of(e: &StkError) -> ErrorEntry {
        ErrorEntry {
            code: e.code_string(),
            domain: e.domain.tag().to_string(),
            title: e.title.clone(),
            details: e.details.clone(),
            action: e.user_action.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WriteRecord {
    pub path: String,
    pub bytes: u64,
    /// written | skipped | would-write（dry-run）
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Envelope {
    pub schema: u32,
    pub tool: &'static str,
    pub version: &'static str,
    pub command: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss: Option<serde_json::Value>,
    #[serde(default)]
    pub writes: Vec<WriteRecord>,
    #[serde(default)]
    pub errors: Vec<ErrorEntry>,
}

impl Envelope {
    pub fn ok(command: &str, data: serde_json::Value) -> Envelope {
        Envelope {
            schema: SCHEMA,
            tool: TOOL,
            version: env!("CARGO_PKG_VERSION"),
            command: command.to_string(),
            ok: true,
            data: Some(data),
            loss: None,
            writes: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn fail(command: &str, e: &StkError) -> Envelope {
        let mut env = Envelope {
            schema: SCHEMA,
            tool: TOOL,
            version: env!("CARGO_PKG_VERSION"),
            command: command.to_string(),
            ok: false,
            data: None,
            loss: None,
            writes: Vec::new(),
            errors: vec![ErrorEntry::of(e)],
        };
        if let Some(action) = &e.user_action {
            env.errors[0].action = Some(action.clone());
        }
        env
    }

    /// 校验类命令：报告本身就是产物。判定不通过时用 `ok=false` + `errors` 表达，
    /// 但 `data` 里的全部发现必须一起交出去 —— 把报告丢掉再退非零，用户拿不到要改的东西。
    pub fn fail_with(command: &str, e: &StkError, data: serde_json::Value) -> Envelope {
        let mut env = Envelope::fail(command, e);
        env.data = Some(data);
        env
    }

    pub fn with_loss(mut self, loss: &core_model::loss::LossReport) -> Envelope {
        self.loss = serde_json::to_value(loss).ok();
        self
    }

    pub fn to_json_line(&self) -> String {
        // 单行 JSON：便于 `| jq` 与日志采集；序列化失败时给出可诊断的兜底而不是 panic
        serde_json::to_string(self).unwrap_or_else(|e| {
            format!(r#"{{"schema":{SCHEMA},"tool":"{TOOL}","ok":false,"errors":[{{"code":"STK-OUT-9001","domain":"OUT","title":"结果序列化失败","details":["{e}"]}}]}}"#)
        })
    }

    /// 人读输出（不带 `--json` 时）。保持简短，机器可解析一律走 `--json`。
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.ok {
            out.push_str(&format!("ok  {}\n", self.command));
            if let Some(d) = &self.data {
                out.push_str(&serde_json::to_string_pretty(d).unwrap_or_default());
                out.push('\n');
            }
            for w in &self.writes {
                out.push_str(&format!("  {} {} bytes ({})\n", w.path, w.bytes, w.state));
            }
        } else {
            for e in &self.errors {
                out.push_str(&format!("{}: {}\n", e.code, e.title));
                for d in &e.details {
                    out.push_str(&format!("  - {d}\n"));
                }
                if let Some(a) = &e.action {
                    out.push_str(&format!("  建议：{a}\n"));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::error::Domain;

    #[test]
    fn envelope_is_single_line_and_stable() {
        let e = Envelope::ok("probe", serde_json::json!({"format": "ass"}));
        let line = e.to_json_line();
        assert!(!line.contains('\n'), "JSONL 友好：一行一个结果");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["tool"], TOOL);
        assert_eq!(v["data"]["format"], "ass");
        assert!(v.get("errors").is_none() || v["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn failure_carries_code_action_and_details() {
        let err = StkError::simple(Domain::Pgs, 95, "超过 255 色")
            .with_detail("frame 0")
            .with_action("需要 Advanced Author 的量化策略");
        let env = Envelope::fail("pgs encode", &err);
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "STK-PGS-0095");
        assert_eq!(env.errors[0].domain, "PGS");
        let text = env.to_text();
        assert!(
            text.contains("STK-PGS-0095") && text.contains("建议"),
            "{text}"
        );
    }

    #[test]
    fn loss_report_is_embedded_when_present() {
        let mut loss = core_model::loss::LossReport::new();
        loss.preserved("时间轴");
        let env = Envelope::ok("hdr", serde_json::json!(null)).with_loss(&loss);
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert_eq!(v["loss"]["preserved"][0], "时间轴");
    }
}
