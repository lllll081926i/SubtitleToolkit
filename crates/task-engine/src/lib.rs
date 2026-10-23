//! 任务引擎（15、38）。三块内容：
//! - 输出事务（07 §7、38 §1）：原文件只读；先写 tmp，fsync/close，重开校验，再 rename，
//!   失败绝不留下半成品覆盖旧文件（AT-OUT-001）；
//! - 命名模板与覆盖策略（15 §5/§6）；
//! - 任务模型（15 §1/§4/§7/§8、38 §2/§3/§6/§7）：`job`（状态与摘要）、`cachekey`（多输出
//!   共享缓存的 key 合同）、`jobstate`（`.jobstate.json` 恢复凭据）。

pub mod cachekey;
pub mod discover;
pub mod job;
pub mod jobstate;
pub mod runner;

use std::path::{Path, PathBuf};

use core_model::error::{Domain, StkError};

pub use cachekey::CacheKey;
pub use cachekey::CacheKeyParts;
pub use cachekey::RenderIdentity;
pub use discover::DiscoverOptions;
pub use discover::Discovered;
pub use discover::Pair;
pub use job::CancelToken;
pub use job::CrashAction;
pub use job::CrashBudget;
pub use job::InputSet;
pub use job::Job;
pub use job::JobOutcome;
pub use job::JobReport;
pub use job::OutputProfile;
pub use job::ProfileOutcome;
pub use jobstate::JobState;
pub use jobstate::Reuse;
pub use jobstate::StageRecord;
pub use runner::Executed;
pub use runner::Permit;
pub use runner::Pools;
pub use runner::Stage;
pub use runner::{run_profiles, run_profiles_observed};

/// 命名模板 token：`{stem}` `{profile}` `{width}` `{height}` `{fps}` `{hdr}` `{ext}`（15 §5）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameTemplate {
    pub pattern: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NameFields {
    pub stem: String,
    pub profile: String,
    pub width: u32,
    pub height: u32,
    pub fps: String,
    pub hdr: String,
    pub ext: String,
}

impl NameTemplate {
    pub fn new(pattern: &str) -> Result<NameTemplate, StkError> {
        // 只允许已知 token，防止 { 未闭合导致产物名出现花括号
        let mut depth = 0usize;
        for c in pattern.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return Err(bad_template("出现未配对的 }"));
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if depth != 0 {
            return Err(bad_template("token 未闭合"));
        }
        Ok(NameTemplate {
            pattern: pattern.to_string(),
        })
    }

    pub const DEFAULT: &'static str = "{stem}.{ext}";

    pub fn render(&self, f: &NameFields) -> Result<String, StkError> {
        if f.stem.is_empty() {
            return Err(bad_template("stem 为空"));
        }
        let ext = sanitize_component(f.ext.as_str())?;
        if ext.is_empty() {
            return Err(bad_template("扩展名为空"));
        }
        let mut out = String::new();
        let mut rest = self.pattern.as_str();
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let close = rest[i..]
                .find('}')
                .ok_or_else(|| bad_template("token 未闭合"))?;
            let token = &rest[i..i + close + 1];
            let val = match token {
                "{stem}" => sanitize_component(&f.stem)?,
                "{profile}" => sanitize_component(&f.profile)?,
                "{width}" => f.width.to_string(),
                "{height}" => f.height.to_string(),
                "{fps}" => sanitize_component(&f.fps)?,
                "{hdr}" => sanitize_component(&f.hdr)?,
                "{ext}" => ext.clone(),
                other => return Err(bad_template(&format!("未知 token {other}"))),
            };
            out.push_str(&val);
            rest = &rest[i + close + 1..];
        }
        out.push_str(rest);
        if out.len() > 255 {
            return Err(bad_template("文件名超过 255 字节"));
        }
        Ok(out)
    }
}

fn bad_template(msg: &str) -> StkError {
    StkError::simple(Domain::Input, 70, "命名模板非法").with_detail(msg.to_string())
}

/// Windows 目录名/文件名禁用字符与保留名。返回去掉禁用字符后的安全片段。
pub fn sanitize_component(s: &str) -> Result<String, StkError> {
    let trimmed = s.trim_matches(|c: char| c == ' ' || c == '.');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err(
            StkError::simple(Domain::Input, 71, "文件名含 Windows 禁用字符")
                .with_detail(trimmed.to_string())
                .with_action("去掉 < > : \" / \\ | ? * 后重试"),
        );
    }
    if trimmed.chars().any(|c| (c as u32) < 32) {
        return Err(StkError::simple(Domain::Input, 71, "文件名含控制字符"));
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let head = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&head.as_str()) {
        return Err(
            StkError::simple(Domain::Input, 72, "文件名命中 Windows 保留名").with_detail(head),
        );
    }
    Ok(trimmed.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverwritePolicy {
    Skip,
    Rename,
    ReplaceAfterVerify,
}

/// 原子输出：写 tmp → flush → drop(关闭) → verify(bytes) → rename。
/// `verify` 返回 Err 时保留 tmp 供诊断、**不动** final（AT-OUT-001）。
pub struct AtomicOutput {
    final_path: PathBuf,
    tmp_path: PathBuf,
    policy: OverwritePolicy,
}

impl AtomicOutput {
    pub fn new(
        final_path: impl AsRef<Path>,
        policy: OverwritePolicy,
    ) -> Result<AtomicOutput, StkError> {
        let final_path = final_path.as_ref().to_path_buf();
        if !final_path.is_absolute() {
            return Err(StkError::simple(
                Domain::Output,
                10,
                "输出路径必须是绝对路径",
            ));
        }
        let name = final_path
            .file_name()
            .ok_or_else(|| StkError::simple(Domain::Output, 11, "输出路径没有文件名"))?
            .to_string_lossy()
            .to_string();
        // 38 §5：临时名带唯一后缀，崩溃后可被扫描识别
        let uniq = format!("{}.{}.tmp", name, uuid_like());
        let tmp_path = final_path.with_file_name(uniq);
        Ok(AtomicOutput {
            final_path,
            tmp_path,
            policy,
        })
    }

    pub fn tmp_path(&self) -> &Path {
        &self.tmp_path
    }
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// 目标已存在时按策略决定：Skip → Skipped；Rename → 换名；Replace → 继续（先留备份）。
    pub fn preflight(&self) -> Result<Preflight, StkError> {
        if !self.final_path.exists() {
            return Ok(Preflight::Write {
                final_path: self.final_path.clone(),
            });
        }
        match self.policy {
            OverwritePolicy::Skip => Ok(Preflight::Skipped),
            OverwritePolicy::Rename => Ok(Preflight::Write {
                final_path: unique_sibling(&self.final_path)?,
            }),
            OverwritePolicy::ReplaceAfterVerify => Ok(Preflight::Write {
                final_path: self.final_path.clone(),
            }),
        }
    }

    /// 写出并在 verify 通过后才替换。verify 失败 → 保留旧文件，tmp 留着诊断。
    pub fn commit<F: FnOnce(&[u8]) -> Result<(), StkError>>(
        &self,
        bytes: &[u8],
        verify: F,
    ) -> Result<Commit, StkError> {
        let target = match self.preflight()? {
            Preflight::Skipped => return Ok(Commit::Skipped),
            Preflight::Write { final_path } => final_path,
        };
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| StkError::from(e).with_detail(dir.display().to_string()))?;
        }
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&self.tmp_path)?;
            f.write_all(bytes)?;
            f.flush()?;
            f.sync_all()
                .map_err(|e| StkError::from(e).with_detail("sync_all 失败"))?;
        } // drop 关闭句柄，rename 才安全

        let read_back = std::fs::read(&self.tmp_path)?;
        if read_back != bytes {
            return Err(
                StkError::simple(Domain::Output, 20, "临时文件回读与内存内容不一致")
                    .with_detail(format!("tmp={}", self.tmp_path.display())),
            );
        }
        // verify 失败时 tmp 是**故意留下的**（供诊断，实现选择见类型注释）；但"这里留了一份
        // 文件"这件事必须写进错误里。留了而不说，等于让用户在输出目录里捡到一个没头没尾的
        // `.tmp` 自己去猜（ISSUES D13：门阶那次失败渲染就是这样，报告只说 BDN 校验不过）。
        if let Err(e) = verify(&read_back) {
            return Err(e
                .with_detail(format!(
                    "目标文件未被替换；这次的内容留在临时文件里供诊断：{}",
                    self.tmp_path.display()
                ))
                .with_action("核对那个临时文件后自行删除它"));
        }

        let backup = if target.exists() && self.policy == OverwritePolicy::ReplaceAfterVerify {
            let backup = backup_path(&target)?;
            std::fs::rename(&target, &backup).map_err(|e| {
                StkError::from(e).with_detail(format!("备份失败 -> {}", backup.display()))
            })?;
            Some(backup)
        } else {
            None
        };
        install_with_rollback(&self.tmp_path, &target, backup.as_deref())?;
        Ok(Commit::Written(target))
    }
}

fn install_with_rollback(tmp: &Path, target: &Path, backup: Option<&Path>) -> Result<(), StkError> {
    if let Err(install_error) = std::fs::rename(tmp, target) {
        let install_detail = format!("rename {} -> {}", tmp.display(), target.display());
        return match backup {
            Some(backup) => match std::fs::rename(backup, target) {
                Ok(()) => Err(StkError::from(install_error).with_detail(format!(
                    "{install_detail}；旧目标已恢复到 {}；新文件仍在临时文件：{}",
                    target.display(),
                    tmp.display()
                ))),
                Err(restore_error) => Err(StkError::from(install_error).with_detail(format!(
                    "{install_detail}；恢复旧目标失败：{restore_error}；旧文件仍在备份：{}；新文件仍在临时文件：{}",
                    backup.display(),
                    tmp.display()
                ))),
            },
            None => Err(StkError::from(install_error).with_detail(install_detail)),
        };
    }
    Ok(())
}

#[derive(Debug)]
pub enum Preflight {
    Skipped,
    Write { final_path: PathBuf },
}
#[derive(Debug)]
pub enum Commit {
    Skipped,
    Written(PathBuf),
}

fn unique_sibling(p: &Path) -> Result<PathBuf, StkError> {
    let name = p
        .file_name()
        .ok_or_else(|| StkError::simple(Domain::Output, 11, "无文件名"))?
        .to_string_lossy()
        .to_string();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for n in 1..1000u32 {
        let cand = p.with_file_name(format!("{stem}({n}){ext}"));
        if !cand.exists() {
            return Ok(cand);
        }
    }
    Err(StkError::simple(
        Domain::Output,
        21,
        "同名文件过多，无法自动改名",
    ))
}

fn backup_path(p: &Path) -> Result<PathBuf, StkError> {
    let name = p
        .file_name()
        .ok_or_else(|| StkError::simple(Domain::Output, 11, "无文件名"))?
        .to_string_lossy()
        .to_string();
    Ok(p.with_file_name(format!("{name}.bak-{}", uuid_like())))
}

/// 不引入 uuid crate 的最小唯一串：时间 + 进程 id + 计数。
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{}-{:x}", t, std::process::id(), n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("stk-out-{tag}-{}", uuid_like()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn template_renders_all_tokens() {
        let t = NameTemplate::new("{stem}.{profile}.{width}x{height}.{fps}.{hdr}.{ext}").unwrap();
        let f = NameFields {
            stem: "movie".into(),
            profile: "BD-SUP".into(),
            width: 1920,
            height: 1080,
            fps: "23.976".into(),
            hdr: "HDR".into(),
            ext: "sup".into(),
        };
        assert_eq!(
            t.render(&f).unwrap(),
            "movie.BD-SUP.1920x1080.23.976.HDR.sup"
        );
    }

    #[test]
    fn default_template_uses_only_supported_tokens() {
        let template = NameTemplate::new(NameTemplate::DEFAULT).unwrap();
        let fields = NameFields {
            stem: "movie".into(),
            ext: "sup".into(),
            ..Default::default()
        };
        assert_eq!(template.render(&fields).unwrap(), "movie.sup");
    }

    #[test]
    fn template_rejects_bad_input() {
        assert!(NameTemplate::new("{stem").is_err());
        assert!(NameTemplate::new("a}b").is_err());
        let t = NameTemplate::new("{stem}.{nope}.{ext}").unwrap();
        let f = NameFields {
            stem: "x".into(),
            ext: "ass".into(),
            ..Default::default()
        };
        assert!(t.render(&f).is_err());
        let empty_stem = NameFields {
            stem: "".into(),
            ext: "ass".into(),
            ..Default::default()
        };
        assert!(NameTemplate::new("{stem}.{ext}")
            .unwrap()
            .render(&empty_stem)
            .is_err());
    }

    #[test]
    fn sanitize_blocks_windows_hazards() {
        assert!(sanitize_component("a<b").is_err());
        assert!(sanitize_component("c:/x").is_err());
        assert!(sanitize_component("CON").is_err());
        assert!(sanitize_component("nul.txt").is_err());
        assert!(sanitize_component("has\ttab").is_err());
        assert_eq!(sanitize_component("  spaced  ").unwrap(), "spaced");
        assert_eq!(sanitize_component("中文字幕").unwrap(), "中文字幕");
        assert_eq!(sanitize_component("").unwrap(), "");
        // 合法但带点的名字不能被误判为保留名
        assert_eq!(sanitize_component("CONsole.ass").unwrap(), "CONsole.ass");
    }

    #[test]
    fn commit_writes_verifies_and_removes_tmp() {
        let dir = tmpdir("ok");
        let target = dir.join("out.srt");
        let out = AtomicOutput::new(&target, OverwritePolicy::Skip).unwrap();
        let c = out.commit(b"hello", |_| Ok(())).unwrap();
        assert!(matches!(c, Commit::Written(_)));
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
        assert!(!out.tmp_path().exists(), "tmp 必须被 rename 消耗掉");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_failure_keeps_old_file_intact() {
        // AT-OUT-001
        let dir = tmpdir("verify");
        let target = dir.join("out.srt");
        std::fs::write(&target, b"OLD").unwrap();
        let out = AtomicOutput::new(&target, OverwritePolicy::ReplaceAfterVerify).unwrap();
        let e = out
            .commit(b"NEW", |_| {
                Err(StkError::simple(Domain::Validate, 9, "自检不过"))
            })
            .unwrap_err();
        assert_eq!(e.code_string(), "STK-VAL-0009");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"OLD",
            "verify 失败绝不能替换旧文件"
        );
        let mut bak_exists = false;
        for e in std::fs::read_dir(&dir).unwrap() {
            let n = e.unwrap().file_name().to_string_lossy().to_string();
            if n.contains(".tmp") {
                bak_exists = true;
            }
        }
        assert!(bak_exists, "失败时的 tmp 应保留以便诊断");
        // 保留是设计（类型注释与上面的断言就是那件事），但**说了才算**：错误里必须点名
        // 留下的那个路径，否则用户在输出目录里看到一个没头没尾的 .tmp（ISSUES D13）
        let details = e.details.join("\n");
        assert!(
            details.contains(&out.tmp_path().display().to_string()),
            "报告没交代留下了哪个临时文件：{details}"
        );
        assert!(
            e.user_action.is_some(),
            "要给出可执行的下一步，而不是只说失败：{e:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn length_mismatch_detected_by_readback() {
        let dir = tmpdir("readback");
        let target = dir.join("x.srt");
        let out = AtomicOutput::new(&target, OverwritePolicy::Skip).unwrap();
        // 用 verify 模拟"重开后发现内容不对"
        let e = out
            .commit(b"data", |b| {
                if b == b"data" {
                    Ok(())
                } else {
                    Err(StkError::simple(Domain::Validate, 1, "不一致"))
                }
            })
            .unwrap();
        assert!(matches!(e, Commit::Written(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skip_policy_leaves_existing() {
        let dir = tmpdir("skip");
        let target = dir.join("a.ass");
        std::fs::write(&target, b"KEEP").unwrap();
        let out = AtomicOutput::new(&target, OverwritePolicy::Skip).unwrap();
        assert!(matches!(
            out.commit(b"NEW", |_| Ok(())).unwrap(),
            Commit::Skipped
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"KEEP");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_policy_never_truncates() {
        let dir = tmpdir("rename");
        let target = dir.join("a.ass");
        std::fs::write(&target, b"KEEP").unwrap();
        let out = AtomicOutput::new(&target, OverwritePolicy::Rename).unwrap();
        let c = out.commit(b"NEW", |_| Ok(())).unwrap();
        match c {
            Commit::Written(p) => {
                assert_ne!(p, target);
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    b"KEEP",
                    "原文件必须未被触碰"
                );
                assert_eq!(std::fs::read(&p).unwrap(), b"NEW");
            }
            Commit::Skipped => panic!("Rename 策略不该 Skip"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replace_policy_backs_up_original() {
        let dir = tmpdir("replace");
        let target = dir.join("a.ass");
        std::fs::write(&target, b"OLD").unwrap();
        let out = AtomicOutput::new(&target, OverwritePolicy::ReplaceAfterVerify).unwrap();
        assert!(matches!(
            out.commit(b"NEW", |_| Ok(())).unwrap(),
            Commit::Written(_)
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failed_install_reports_backup_when_rollback_is_blocked() {
        let dir = tmpdir("rollback");
        let tmp = dir.join("new.tmp");
        let target = dir.join("out.srt");
        let backup = dir.join("out.srt.bak-test");
        std::fs::write(&tmp, b"NEW").unwrap();
        std::fs::write(&backup, b"OLD").unwrap();
        std::fs::create_dir(&target).unwrap();

        let error = install_with_rollback(&tmp, &target, Some(&backup)).unwrap_err();
        let details = error.details.join("\n");
        assert!(details.contains(&backup.display().to_string()), "{details}");
        assert!(details.contains(&tmp.display().to_string()), "{details}");
        assert_eq!(std::fs::read(&backup).unwrap(), b"OLD");
        assert!(tmp.is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_output_path_is_rejected() {
        assert!(AtomicOutput::new("out.srt", OverwritePolicy::Skip).is_err());
    }

    #[test]
    fn long_path_chinese_and_emoji() {
        let deep = "深".repeat(60) + "😀";
        let dir = tmpdir("longpath")
            .join(deep.as_str())
            .join(deep.as_str())
            .join(deep.as_str());
        let target = dir.join("字幕😀.ass");
        let made = std::fs::create_dir_all(&dir);
        assert!(made.is_ok(), "create_dir_all 失败：{:?}", made.err());
        assert!(
            target.display().to_string().len() > 260,
            "本用例必须真的超过 260 字符才有意义"
        );
        let out = AtomicOutput::new(&target, OverwritePolicy::Skip).unwrap();
        let committed = out.commit(b"[Script Info]\n", |_| Ok(()));
        assert!(committed.is_ok(), "commit 失败：{:?}", committed.err());
        let read = std::fs::read(&target);
        assert!(read.is_ok(), "回读失败：{:?}", read.err());
        assert_eq!(read.unwrap(), b"[Script Info]\n");
    }
}
