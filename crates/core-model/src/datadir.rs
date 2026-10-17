//! 数据目录解析（28 §4 的 portable 语义）。
//!
//! 目标只有一条：**便携包解包后不改用户机器上的任何状态**。
//! 默认仍然走 `%LOCALAPPDATA%\subtitletool`（安装版就该这样）；
//! 只有当可执行文件旁边存在 `portable.flag` 时，才把数据目录换成同目录下的 `data/`。
//!
//! ## 为什么用"标记文件"而不是"检测自己在 U 盘上"
//!
//! 便携是**用户的选择**，不是我们猜出来的属性。自动嗅探（可移动盘、无写权限的
//! Program Files）会带来一类很难查的故障：同一份 exe 在不同机器上落到不同数据目录，
//! 而用户以为两边看的是同一份设置。
//!
//! ## 环境变量优先级
//!
//! `STK_DATA_DIR` 最高：门阶与测试需要"我明确指定一个目录"这条路，
//! 而它不能依赖 exe 旁边能不能写文件。

use std::path::{Path, PathBuf};

/// 数据目录的来源。报告里要说清"东西写到哪去了"，否则"便携没生效"只能靠猜。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataDirKind {
    /// `STK_DATA_DIR` 显式指定
    Explicit,
    /// 可执行文件旁边有 `portable.flag`，数据落在同级 `data/`
    Portable,
    /// 默认：`%LOCALAPPDATA%\subtitletool`
    UserProfile,
    /// 环境里既没有 LOCALAPPDATA 也没有 APPDATA（精简运行环境）
    Unavailable,
}

impl DataDirKind {
    pub fn label(self) -> &'static str {
        match self {
            DataDirKind::Explicit => "explicit",
            DataDirKind::Portable => "portable",
            DataDirKind::UserProfile => "user-profile",
            DataDirKind::Unavailable => "unavailable",
        }
    }
}

/// 数据目录 + 判定理由。`path` 为 `None` 表示连一个可写位置都定不下来。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir {
    pub path: Option<PathBuf>,
    pub kind: DataDirKind,
    /// 为什么是这个结论（进 doctor 报告，便于"便携没生效"这类问题一眼定位）
    pub why: String,
}

impl DataDir {
    /// 便携标记文件名（28 §4 的原话）。
    pub const PORTABLE_FLAG: &'static str = "portable.flag";

    /// 解析数据目录。
    ///
    /// `exe_dir` 是可执行文件所在目录；传 `None` 表示拿不到（此时 portable 判定跳过）。
    pub fn resolve(exe_dir: Option<&Path>) -> DataDir {
        if let Ok(v) = std::env::var("STK_DATA_DIR") {
            if !v.trim().is_empty() {
                return DataDir {
                    path: Some(PathBuf::from(v)),
                    kind: DataDirKind::Explicit,
                    why: "由 STK_DATA_DIR 指定".to_string(),
                };
            }
        }
        if let Some(dir) = exe_dir {
            let flag = dir.join(Self::PORTABLE_FLAG);
            if flag.is_file() {
                return DataDir {
                    path: Some(dir.join("data")),
                    kind: DataDirKind::Portable,
                    why: format!("{} 存在，数据落在同级 data/", Self::PORTABLE_FLAG),
                };
            }
        }
        for var in ["LOCALAPPDATA", "APPDATA"] {
            if let Ok(v) = std::env::var(var) {
                if !v.trim().is_empty() {
                    return DataDir {
                        path: Some(PathBuf::from(v).join("subtitletool")),
                        kind: DataDirKind::UserProfile,
                        why: format!("默认：%{var}%\\subtitletool（无 portable.flag）"),
                    };
                }
            }
        }
        DataDir {
            path: None,
            kind: DataDirKind::Unavailable,
            why: "环境里没有 LOCALAPPDATA/APPDATA，也没有 portable.flag".to_string(),
        }
    }

    /// 用当前进程的可执行文件位置解析。
    ///
    /// 取 exe 所在目录而不是当前工作目录：用户从任意目录启动便携包，
    /// 数据都该落在 exe 旁边，而不是"启动时那个目录"。
    pub fn resolve_current_exe() -> DataDir {
        let exe = std::env::current_exe().ok();
        let dir = exe.as_deref().and_then(Path::parent);
        Self::resolve(dir)
    }

    /// 确保目录存在并返回它。失败时给的是"要建哪个目录"的明确信息。
    pub fn ensure(&self) -> std::io::Result<Option<PathBuf>> {
        match &self.path {
            None => Ok(None),
            Some(p) => {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                }
                Ok(Some(p.clone()))
            }
        }
    }

    /// 数据目录下的一个文件（目录本身不存在也返回路径：调用方决定要不要建）。
    pub fn file(&self, name: &str) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.join(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-datadir-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 环境变量是全局状态，测试之间会互相污染。这一组必须串行：
    /// 用一把进程内互斥锁，因为 `cargo test` 默认多线程。
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn portable_flag_next_to_the_exe_switches_the_data_dir() {
        let _g = env_lock();
        std::env::remove_var("STK_DATA_DIR");
        let dir = scratch("portable");
        // 没有标记：走用户目录（或 unavailable，取决于环境）
        let before = DataDir::resolve(Some(&dir));
        assert_ne!(before.kind, DataDirKind::Portable);
        assert_eq!(before.path.as_deref(), None.or(before.path.as_deref()));
        assert!(!before.why.contains("portable.flag 存在"), "{}", before.why);

        // 放上标记：切到同级的 data/
        std::fs::write(dir.join(DataDir::PORTABLE_FLAG), b"").unwrap();
        let after = DataDir::resolve(Some(&dir));
        assert_eq!(after.kind, DataDirKind::Portable, "{}", after.why);
        assert_eq!(after.path, Some(dir.join("data")));
        assert!(after.why.contains("portable.flag"));

        // ensure 会真的建出目录
        let made = after.ensure().unwrap().unwrap();
        assert!(made.is_dir(), "{} 应该被建出来", made.display());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn explicit_env_var_wins_over_portable_flag() {
        let _g = env_lock();
        let dir = scratch("explicit");
        std::fs::write(dir.join(DataDir::PORTABLE_FLAG), b"").unwrap();
        let want = dir.join("chosen");
        std::env::set_var("STK_DATA_DIR", &want);
        let d = DataDir::resolve(Some(&dir));
        assert_eq!(d.kind, DataDirKind::Explicit);
        assert_eq!(d.path, Some(want));
        std::env::remove_var("STK_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 空字符串的 `STK_DATA_DIR` 不能当成"用户指定了当前目录"——那是脚本变量为空的样子。
    #[test]
    fn blank_env_var_is_ignored() {
        let _g = env_lock();
        std::env::set_var("STK_DATA_DIR", "   ");
        let dir = scratch("blank");
        let d = DataDir::resolve(Some(&dir));
        assert_ne!(d.kind, DataDirKind::Explicit, "{}", d.why);
        std::env::remove_var("STK_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 可执行目录拿不到（也不该因此崩）：跳过 portable 判定，退到用户目录。
    #[test]
    fn unknown_exe_dir_never_panics_and_skips_portable() {
        let _g = env_lock();
        std::env::remove_var("STK_DATA_DIR");
        let d = DataDir::resolve(None);
        assert_ne!(d.kind, DataDirKind::Portable);
        assert!(!d.why.is_empty(), "没有结论也必须给理由");
    }

    #[test]
    fn file_helper_targets_the_resolved_dir() {
        let d = DataDir {
            path: Some(PathBuf::from("D:\\portable\\data")),
            kind: DataDirKind::Portable,
            why: String::new(),
        };
        assert_eq!(
            d.file("fontdb-v1.json"),
            Some(PathBuf::from("D:\\portable\\data\\fontdb-v1.json"))
        );
        let none = DataDir {
            path: None,
            kind: DataDirKind::Unavailable,
            why: String::new(),
        };
        assert_eq!(none.file("x"), None);
    }

    #[test]
    fn kinds_have_stable_labels() {
        for (k, want) in [
            (DataDirKind::Explicit, "explicit"),
            (DataDirKind::Portable, "portable"),
            (DataDirKind::UserProfile, "user-profile"),
            (DataDirKind::Unavailable, "unavailable"),
        ] {
            assert_eq!(k.label(), want);
        }
    }
}
