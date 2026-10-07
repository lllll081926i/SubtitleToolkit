//! `.jobstate.json`：崩溃/断电后的恢复凭据（15 §7、38 §5/§6）。
//!
//! 规则只有一条：**哈希与元数据都验证通过的产物才能复用**。
//! 复用"看起来在"的文件等于把半成品当成成品，比失败更糟。
//! 工具链版本记录在同一份状态里，版本变化会让相关缓存整体失效（38 §6）。

use std::path::Path;

use core_model::error::{Domain, Result, StkError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    /// 15 §1：一个 job 有多个输入，同一个 profile 在每个输入上都会跑一次。
    /// 只按 profile 记账会让第二个输入的哈希覆盖第一个，于是第一个输入"复用到别人的产物"。
    pub input: String,
    pub profile: String,
    /// 停在哪个阶段：parse/render/author/write/verify
    pub stage: String,
    /// 产物路径（仍在磁盘上时才谈复用）
    pub path: String,
    pub artifact_sha256: String,
    /// 15 §4 的缓存 key：分量（含 renderer 身份）变了就不许复用
    pub cache_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobState {
    pub job_id: String,
    /// ("libass", "0.17.3+<sha>") 之类的依赖版本快照（01 §任务凭据、38 §6）
    pub tool_versions: Vec<(String, String)>,
    pub stages: Vec<StageRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reuse {
    /// 可以复用：直接跳过重算
    Yes,
    /// 该 profile 没有已完成记录
    UnknownProfile,
    /// 缓存 key 变了（renderer/字体/画布/fps 等分量变化）
    CacheKeyChanged,
    /// 产物哈希对不上：文件被改、被删、或上次根本没写完
    ArtifactMismatch,
    /// 工具链版本变了
    ToolchainChanged,
}

impl Reuse {
    pub fn may_reuse(self) -> bool {
        self == Reuse::Yes
    }

    pub fn reason(self) -> &'static str {
        match self {
            Reuse::Yes => "可复用",
            Reuse::UnknownProfile => "没有该 profile 的已完成记录",
            Reuse::CacheKeyChanged => "缓存 key 与当前分量不一致",
            Reuse::ArtifactMismatch => "产物哈希与记录不一致（文件被改或上次未写完）",
            Reuse::ToolchainChanged => "依赖版本变化，缓存整体失效",
        }
    }
}

impl JobState {
    pub fn new(job_id: impl Into<String>, tool_versions: Vec<(String, String)>) -> JobState {
        JobState {
            job_id: job_id.into(),
            tool_versions,
            stages: Vec::new(),
        }
    }

    /// 记录一次"验证通过的产物"。同 profile 重复记录时以最后一次为准
    /// （重试成功后不该还留着失败那次的哈希）。
    pub fn record(&mut self, r: StageRecord) -> Result<()> {
        for (field, v) in [
            ("artifact_sha256", &r.artifact_sha256),
            ("cache_key", &r.cache_key),
        ] {
            if v.len() != 64 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(StkError::simple(
                    Domain::Output,
                    40,
                    "恢复状态里的哈希必须是 sha256 hex",
                )
                .with_detail(format!("{field}={v:?}")));
            }
        }
        if r.input.trim().is_empty()
            || r.profile.trim().is_empty()
            || r.stage.trim().is_empty()
            || r.path.trim().is_empty()
        {
            return Err(StkError::simple(
                Domain::Output,
                41,
                "恢复状态记录字段不完整",
            ));
        }
        self.stages
            .retain(|s| s.input != r.input || s.profile != r.profile);
        self.stages.push(r);
        Ok(())
    }

    /// 判断某个 profile 的产物能否复用。
    /// `actual_sha256` 由调用方**真的去读一遍文件**算出来；传 `None` 表示文件不在了。
    pub fn reuse_for(
        &self,
        input: &str,
        profile: &str,
        want_cache_key: &str,
        actual_sha256: Option<&str>,
        current_tool_versions: &[(String, String)],
    ) -> Reuse {
        if self.tool_versions != current_tool_versions {
            return Reuse::ToolchainChanged;
        }
        let Some(rec) = self
            .stages
            .iter()
            .find(|s| s.input == input && s.profile == profile)
        else {
            return Reuse::UnknownProfile;
        };
        if rec.cache_key != want_cache_key {
            return Reuse::CacheKeyChanged;
        }
        match actual_sha256 {
            Some(hash) if hash == rec.artifact_sha256 => Reuse::Yes,
            _ => Reuse::ArtifactMismatch,
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| {
            StkError::simple(Domain::Output, 42, "jobstate 序列化失败").with_detail(e.to_string())
        })
    }

    pub fn from_json(s: &str) -> Result<JobState> {
        serde_json::from_str(s).map_err(|e| {
            StkError::simple(Domain::Output, 43, "jobstate 无法解析")
                .with_detail(e.to_string())
                .with_action("状态文件损坏时宁可全部重算，也不要复用未知产物")
        })
    }

    /// 写：走"同目录临时文件 + sync + 回读校验 + rename"的输出事务（38 §1、AT-OUT-001）。
    /// 恢复凭据本身如果是半成品，整条恢复逻辑就失去意义，所以这里不写简化路径。
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let text = self.to_json()?;
        let out = crate::AtomicOutput::new(path, crate::OverwritePolicy::ReplaceAfterVerify)?;
        out.commit(text.as_bytes(), |read_back| {
            // 复解析一遍：写进去的必须真的读得回来
            let s = std::str::from_utf8(read_back).map_err(|e| {
                StkError::simple(Domain::Output, 44, "jobstate 回读不是 UTF-8")
                    .with_detail(e.to_string())
            })?;
            JobState::from_json(s).map(|_| ())
        })?;
        Ok(())
    }

    pub fn read_from(path: &Path) -> Result<JobState> {
        let bytes = std::fs::read(path).map_err(|e| {
            StkError::simple(Domain::Input, 84, "读不到 jobstate 文件")
                .with_detail(format!("{path:?}：{e}"))
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            StkError::simple(Domain::Input, 85, "jobstate 不是合法 UTF-8")
                .with_detail(e.to_string())
        })?;
        JobState::from_json(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(profile: &str) -> StageRecord {
        StageRecord {
            input: "a.ass".into(),
            profile: profile.into(),
            stage: "verify".into(),
            path: format!("out/{profile}"),
            artifact_sha256: "a".repeat(64),
            cache_key: "b".repeat(64),
        }
    }

    fn state() -> JobState {
        let mut s = JobState::new("j1", vec![("libass".into(), "0.17.3".into())]);
        s.record(rec("SUP")).unwrap();
        s
    }

    const TOOLS: &[(&str, &str)] = &[("libass", "0.17.3")];

    fn tools() -> Vec<(String, String)> {
        TOOLS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reuse_requires_matching_hash_and_cache_key() {
        let s = state();
        let want_key = "b".repeat(64);
        assert_eq!(
            s.reuse_for("a.ass", "SUP", &want_key, Some(&"a".repeat(64)), &tools()),
            Reuse::Yes
        );
        assert_eq!(
            s.reuse_for("a.ass", "SUP", &want_key, Some(&"9".repeat(64)), &tools()),
            Reuse::ArtifactMismatch,
            "文件被改过就不能复用"
        );
        assert_eq!(
            s.reuse_for("a.ass", "SUP", &want_key, None, &tools()),
            Reuse::ArtifactMismatch,
            "文件不在就不能复用"
        );
        assert_eq!(
            s.reuse_for(
                "a.ass",
                "SUP",
                &"c".repeat(64),
                Some(&"a".repeat(64)),
                &tools()
            ),
            Reuse::CacheKeyChanged
        );
        assert_eq!(
            s.reuse_for("a.ass", "BDN", &want_key, Some(&"a".repeat(64)), &tools()),
            Reuse::UnknownProfile
        );
    }

    #[test]
    fn toolchain_change_invalidates_even_if_hashes_match() {
        // 38 §6：外部程序版本改变 -> 相关缓存失效
        let s = state();
        let mut newer = tools();
        newer[0].1 = "0.18.0".into();
        let r = s.reuse_for(
            "a.ass",
            "SUP",
            &"b".repeat(64),
            Some(&"a".repeat(64)),
            &newer,
        );
        assert_eq!(r, Reuse::ToolchainChanged);
        assert!(!r.may_reuse());
        assert!(r.reason().contains("版本"));
    }

    #[test]
    fn json_roundtrip_rejects_unknown_fields_and_bad_hashes() {
        let s = state();
        let text = s.to_json().unwrap();
        let back = JobState::from_json(&text).unwrap();
        assert_eq!(s, back);

        let tampered = text.replace("\"stage\"", "\"stageX\"");
        assert!(JobState::from_json(&tampered).is_err(), "未知字段必须拒绝");

        let mut bad = rec("X");
        bad.artifact_sha256 = "not-hex".into();
        let e = s.clone().record(bad).unwrap_err();
        assert_eq!(e.code_string(), "STK-OUT-0040");
    }

    #[test]
    fn two_inputs_with_the_same_profile_do_not_overwrite_each_other() {
        // 这是 batch 的真实形状：一个 job × N 输入 × M profile。
        // 只按 profile 记账时，第二个输入的哈希会盖掉第一个，
        // 于是第一个输入会拿着"别人的哈希"判定为可复用（交付错文件）。
        let mut s = JobState::new("j", tools());
        let mut a = rec("SUP");
        a.input = "a.ass".into();
        a.artifact_sha256 = "a".repeat(64);
        let mut b = rec("SUP");
        b.input = "b.ass".into();
        b.artifact_sha256 = "b".repeat(64);
        s.record(a).unwrap();
        s.record(b).unwrap();
        assert_eq!(s.stages.len(), 2, "两个输入的同一 profile 必须各占一条");
        let key = "b".repeat(64);
        assert_eq!(
            s.reuse_for("a.ass", "SUP", &key, Some(&"a".repeat(64)), &tools()),
            Reuse::Yes,
            "第一个输入要用它自己的记录"
        );
        assert_eq!(
            s.reuse_for("b.ass", "SUP", &key, Some(&"b".repeat(64)), &tools()),
            Reuse::Yes
        );
        assert_eq!(
            s.reuse_for("a.ass", "SUP", &key, Some(&"b".repeat(64)), &tools()),
            Reuse::ArtifactMismatch,
            "拿另一个输入的哈希来判可复用，就是把别人的产物当成本次结果"
        );
        assert_eq!(
            s.reuse_for("c.ass", "SUP", &key, None, &tools()),
            Reuse::UnknownProfile
        );
    }

    #[test]
    fn re_record_replaces_the_previous_attempt() {
        let mut s = state();
        let mut again = rec("SUP");
        again.artifact_sha256 = "d".repeat(64);
        s.record(again).unwrap();
        assert_eq!(s.stages.len(), 1, "重试成功后不该留着上一次记录");
        assert_eq!(
            s.reuse_for(
                "a.ass",
                "SUP",
                &"b".repeat(64),
                Some(&"d".repeat(64)),
                &tools()
            ),
            Reuse::Yes
        );
    }

    #[test]
    fn write_and_read_use_the_atomic_transaction_path() {
        let dir = std::env::temp_dir().join(format!("stk-jobstate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("job.state.json");
        let s = state();
        s.write_to(&path).unwrap();
        let back = JobState::read_from(&path).unwrap();
        assert_eq!(s, back);
        // 残留临时文件不该留下
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .all(|e| !e.file_name().to_string_lossy().contains(".tmp")),
            "事务写完不该留临时文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
