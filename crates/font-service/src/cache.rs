//! 字体库的磁盘缓存（还 AI5 的性能债：每次起一个 CLI 进程都要重扫整座字体库，
//! release 档 ~2.7 秒、debug 档 ~45 秒，门阶因此从 5 分钟涨到 20 分钟）。
//!
//! 缓存的是**索引**（解析出来的名字/字重/轴），不是字节：字节永远在使用点重读。
//!
//! 关键的设计约束是：**缓存只可能让我们慢，不可能让我们答错**，靠的是两层各自的失效判据。
//! * 第一层是这里指纹：`文件名 + 大小 + mtime`。它会骗人（同长度覆盖安装、mtime 被回拨），
//!   所以不能单独当结论用；
//! * 第二层在使用点：`FontDb::missing_glyphs` 判 cmap 覆盖时本来就要重读那个 face 的文件，
//!   顺手把内容哈希与记录比一遍（不一致报 `FONT_CHANGED_AFTER_SCAN`）。凡是进了结论的 face，
//!   字节都是本次进程真读过的。
//!
//! 为什么不做成"命中前把整座库重读复核一遍"：本机实测复核 154 个文件要 2.63 s，
//! 而一次全量解析总共 2.69 s —— 读文件才是大头，复核掉的就是缓存唯一省下的那部分。
//! 代价要如实说：没被用到的 face，它的名字/字重仍是上一次的索引，
//! 于是"哪一条 face 胜出"这个判断在"同长度且同 mtime 的原地覆盖"这种几乎不会发生
//! （Windows 装字体是拷新文件）的情形下可能滞后一次。要完全绕开就 `--no-font-cache`。
//!
//! 缓存不可用（不存在、版本不符、JSON 坏了、权限）一律退回全量扫描：
//! 那是性能问题，不是交付问题，不能让它变成"命令跑不了"。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::{FaceRecord, FontDb, ScanNote};

/// 缓存格式版本。记录结构一变就必须换值，让旧缓存自然作废。
pub const CACHE_VERSION: &str = "stk-fontdb-v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CachedDb {
    pub version: String,
    /// 来源指纹：排序后的 `目录|文件|大小|mtime`，外加"读没读注册表"
    pub fingerprint: Vec<String>,
    pub roots: Vec<String>,
    pub files_read: usize,
    pub registry_only: usize,
    pub faces: Vec<FaceRecord>,
    pub notes: Vec<ScanNote>,
}

/// 缓存文件的位置。测试与门阶用 `STK_FONTCACHE` 指到临时目录，
/// 免得用户的机器上留下跨运行状态、也免得两条梯子互相看到对方的缓存。
pub fn cache_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_FONTCACHE") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("APPDATA"))
        .map(PathBuf::from)
        .ok()?;
    Some(base.join("subtitletool").join("fontdb-v1.json"))
}

/// 由"这次要扫哪些文件"算指纹。这里只 stat，不读内容 —— 便宜就是全部目的。
pub fn fingerprint(files: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .map(|p| {
            let md = std::fs::metadata(p).ok();
            let (len, mtime) = match md {
                Some(m) => {
                    let t = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(-1);
                    (m.len() as i64, t)
                }
                None => (-1, -1),
            };
            format!("{}|{len}|{mtime}", p.display())
        })
        .collect();
    out.sort();
    out
}

/// 读缓存。任何不匹配/读不懂都返回 `None`（调用方退回全量扫描），不报错。
pub fn load(expected: &[String]) -> Option<CachedDb> {
    let path = cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let db: CachedDb = serde_json::from_slice(&bytes).ok()?;
    if db.version != CACHE_VERSION || db.fingerprint != expected {
        return None;
    }
    if db.faces.is_empty() {
        // 空缓存要么是扫描失败要么是写坏了；退回全量扫描，别把"没有字体"当真值用
        return None;
    }
    Some(db)
}

pub fn store(expected: &[String], db: &FontDb) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        // 目录建不出来就放弃缓存：这是性能路径，不值得为它让命令失败
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = CachedDb {
        version: CACHE_VERSION.to_string(),
        fingerprint: expected.to_vec(),
        roots: db.roots.clone(),
        files_read: db.files_read,
        registry_only: db.registry_only,
        faces: db.faces.clone(),
        notes: db.notes.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        // 先写临时文件再改名：半截文件会被上一次 load 的 JSON 校验挡掉，但没必要制造那种窗口
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// 把缓存里的 face 恢复成一份 `FontDb`。
pub fn to_db(cached: &CachedDb) -> FontDb {
    FontDb {
        faces: cached.faces.clone(),
        notes: cached.notes.clone(),
        roots: cached.roots.clone(),
        files_read: cached.files_read,
        registry_only: cached.registry_only,
        cache: crate::db::CacheState::Hit,
    }
}
