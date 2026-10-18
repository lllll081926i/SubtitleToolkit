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
    /// 22 §2 的 correlation id：本次 Job 的 UUID。
    ///
    /// 它必须出现在**每一条**信封里（成功与失败都含），因为它的用途是
    /// “把用户看到的这一次执行与日志里的那几行对上”。
    /// 只在失败时给就不够了：用户可能因为“跑得太慢”而想去找那一次。
    pub corr_id: String,
    /// 这个 id 是**命令自己**定的还是 `Envelope` 随手生成的。
    ///
    /// 区分这两者是必需的：批量路径会生成一个 id 并用它的子 id 标识每个输出，
    /// 那时信封上的 id 必须服从命令给的（否则报告与信封会是两个值）。
    /// 不序列化：它是构造状态，不是契约的一部分。
    #[serde(skip)]
    pub came_with_corr_id: bool,
    /// 命令已经**自己**往 stdout 写了东西，不要让 `emit` 再写一遍。
    ///
    /// ## 为什么需要这个
    ///
    /// `preset example` 的全部用途是“把样板原样拿走存成文件”
    /// （`subtitletool preset example > my.toml`）。
    /// 而 `emit` 默认会再写一份 JSON 信封 —— 实测结果是存出来的文件
    /// 前半是 TOML、后半是 JSON，再拿回来 `validate` 报
    /// “第 20 行 key 没有值”。
    ///
    /// 把这件事做成一个**显式标记**而不是“看命令名特判”：
    /// 特判会散在 `emit` 里，而看到 `emit` 的人不知道哪几个命令是特例。
    #[serde(skip)]
    pub raw_stdout: bool,
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
            corr_id: core_model::CorrId::new_job().as_string(),
            came_with_corr_id: false,
            raw_stdout: false,
            ok: true,
            data: Some(data),
            loss: None,
            writes: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// 命令已经自己往 stdout 写了内容，不要再写信封。
    ///
    /// 只用于 `preset example` 这类“输出就是产物本身”的命令。
    /// 用它的命令必须自己保证退出码正确（`ok` 为 true 时 `emit` 退 0）。
    pub fn ok_quiet(command: &str) -> Envelope {
        let mut e = Envelope::ok(command, serde_json::Value::Null);
        e.raw_stdout = true;
        e
    }

    /// 用给定的 corr_id 构造成功信封。
    ///
    /// 批量路径用这个：一次 batch 的全部输出必须共享**同一个** Job id，
    /// 否则用户在日志里看到几条记录也不知道它们是不是同一次执行（22 §2）。
    pub fn ok_with_id(command: &str, data: serde_json::Value, corr_id: &str) -> Envelope {
        let mut e = Envelope::ok(command, data);
        e.corr_id = corr_id.to_string();
        e
    }

    pub fn fail(command: &str, e: &StkError) -> Envelope {
        let mut env = Envelope {
            schema: SCHEMA,
            tool: TOOL,
            version: env!("CARGO_PKG_VERSION"),
            command: command.to_string(),
            corr_id: core_model::CorrId::new_job().as_string(),
            came_with_corr_id: false,
            raw_stdout: false,
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

    /// 换成指定的 correlation id（22 §2）。
    ///
    /// `ok`/`fail` 里各生成一个只是为了让单独构造的信封也**总有**一个 id；
    /// 真正发出去的那一份一律由 `dispatch` 用同一个值覆盖，
    /// 否则日志与报告会是两个不同的 id。
    ///
    /// 带了 `came_with_corr_id = true` 的用 [`Envelope::claim_corr_id`]：
    /// 后者表示“这个 id 是命令自己定的，输出 id 都以它为父”。
    pub fn with_corr_id(mut self, id: &str) -> Envelope {
        self.corr_id = id.to_string();
        self
    }

    /// 声明“这个 id 由命令自己生成”，并返回自己。
    /// 批量路径用它：它的子 id 已经按这个共享段生成好了，
    /// 改成别的值会让 `output_ids` 与 `corr_id` 分成两家。
    pub fn claim_corr_id(mut self, id: &str) -> Envelope {
        self.corr_id = id.to_string();
        self.came_with_corr_id = true;
        self
    }

    pub fn to_json_line(&self) -> String {
        // 单行 JSON：便于 `| jq` 与日志采集；序列化失败时给出可诊断的兜底而不是 panic
        serde_json::to_string(self).unwrap_or_else(|e| {
            // 兜底里也要带 corr_id：用户拿到的唯一可用线索就是它（22 §2）。
            format!(
                r#"{{"schema":{SCHEMA},"tool":"{TOOL}","corr_id":"{}","ok":false,"errors":[{{"code":"STK-OUT-9001","domain":"OUT","title":"结果序列化失败","details":["{e}"]}}]}}"#,
                self.corr_id
            )
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

    /// 人读输出末尾那行 correlation id。
    ///
    /// 22 §2 要求“用户看到的错误卡可复制 ID”，所以失败时把它单独一行打印出来，
    /// 而不是埋在 JSON 里。成功时也打：用户可能因为“跑得太慢”想去找那一次。
    /// 一行、无修饰、不含空格与路径——抄下来就能用（见 `corrid` 的字符集约束）。
    pub fn corr_line(&self) -> String {
        format!("corr-id: {}", self.corr_id)
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
