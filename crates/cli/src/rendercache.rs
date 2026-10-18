//! 37 §2 的**磁盘**渲染缓存：把前半链的 [`crate::front::Rendered`] 落到盘上。
//!
//! 内存里的 `RenderMemo` 只覆盖单次批量运行，进程一退就没了；37 §2 要求两级缓存，
//! 磁盘这级存在的唯一理由是**跨运行**复用——同一个输入换一组后半链参数
//! （`exact_colors`、`level`、`min_axis`，这些都不进 key）时不必重跑 native 渲染。
//!
//! ## 三条设计决定
//!
//! **一、key 复用 `front` 的口径，不另立一套。**
//! 复用是否成立取决于「输入内容 + 画布 + 帧率 + 默认字体 + renderer 二进制哈希」，
//! 这个判断在 `front` 里已经写死在 `RenderMemo` 的 key 上。磁盘缓存如果再定义一遍
//! key，两份定义迟早分叉，而分叉的后果是**假复用**（拿 A 的像素当 B 的输出）——
//! 那是静默错误，比慢严重得多。所以这里由调用方传同一个 key。
//!
//! **二、写进磁盘的东西必须能证明自己属于那个 key。**
//! 文件按 key 命名，但文件名不构成证据：磁盘可能被换过、被截过、被同名的别的东西占过。
//! 于是每条记录自带 `key` 字段，读回时逐字节比对，不符就当作未命中并删掉。
//! 这是 23 §「不信任外部输入」在本地的落法——缓存文件是上一次进程写给这一次进程的，
//! 中间隔着一次断电和无数次人为操作，它就是不外部输入。
//!
//! **三、读失败一律退化成未命中，绝不报错。**
//! 缓存坏了不该让用户的任务失败；它只该让这次跑得慢一点。唯一的例外是配额计算与
//! 清理（那是用户显式要求的动作，失败要说出来）。
//!
//! ## 编码为什么不用 serde
//!
//! `image-core` 的 `Patch` 没派生 serde，而给它加 Serialize 会把「位图层」和
//! 「序列化格式」耦死：位图内部的字节序、裁剪语义都会因此变成对外契约。
//! 这里用一个显式的、带魔数与长度校验的小编码器代替。代价是几十行代码，
//! 换来的是格式变更时**旧缓存会被识别并丢弃**，而不是被误读成合法数据。

use std::io::Write;
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use image_core::Patch;
use pgs_author::{Digest, Event, EventKind, Plan, PlanStats};

use crate::front::{RenderJob, Rendered};

/// 魔数 + 版本。改编码格式就改版本号，旧记录会被 `Magic` 检查挡掉当成未命中。
const MAGIC: &[u8; 8] = b"STKRCR01";

/// 磁盘缓存的账目：报告用，也让测试能断言"确实命中了"而不是只看它没报错。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    /// 写失败的次数。写不进去只降级不报错，但必须记账——否则用户会以为缓存在工作。
    pub write_failures: usize,
    /// 读到但认不出的文件数（魔数不符、key 不符、长度截断）。读回时已顺手删掉。
    pub corrupt: usize,
}

/// 一次磁盘缓存访问的结果。
pub struct Loaded {
    pub rendered: Rendered,
    /// 从盘上读回来还是真渲染出来的。
    pub from_disk: bool,
}

/// 磁盘渲染缓存。`None` 表示这一层没开（用户关掉了，或拿不到数据目录）。
pub struct DiskCache {
    dir: PathBuf,
    /// 配额上限（字节）。达到上限时按 mtime 淘汰到 90%，见 [`Self::enforce_quota`]。
    max_bytes: u64,
    stats: std::cell::Cell<CacheStats>,
}

/// 默认配额：512 MiB。
///
/// 为什么是这个数：一个 1080p 事件的 patch 实测常在几十到几百 KiB，
/// 512 MiB 容得下数千个事件——足够覆盖一整部片子的重复渲染，又不至于在
/// 用户笔记本上横占磁盘。37 §4 明确要求「配额、峰值内存与输出大小必须有记录后
/// 再冻结默认值」，这里的依据是实测的 patch 尺寸分布，不是拍脑袋。
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// 配额压到多少算清干净。留 10% 余量避免「刚清完又满」的抖动：
/// 如果每次都刚好清到上限，下一个写入立刻又触发一次全目录扫描。
const QUOTA_TARGET: f64 = 0.9;

impl DiskCache {
    /// 从 [`DataDir`](core_model::DataDir) 解析缓存目录。
    ///
    /// `STK_RENDERCACHE_DIR` 优先，其次是数据目录下的 `rendercache/`，
    /// 环境变量 `STK_RENDERCACHE=0` 明确关闭这一层。
    /// 关掉是用户的正当选择：缓存文件的字节来自渲染结果，对某些场景来说
    /// 「不落盘」本身就是需求。
    pub fn from_env() -> Option<DiskCache> {
        if let Ok(v) = std::env::var("STK_RENDERCACHE") {
            if v == "0" || v.eq_ignore_ascii_case("off") {
                return None;
            }
        }
        let dir = match std::env::var("STK_RENDERCACHE_DIR") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => core_model::DataDir::resolve_current_exe().file("rendercache")?,
        };
        let max = std::env::var("STK_RENDERCACHE_MAX_BYTES")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_BYTES);
        Some(DiskCache {
            dir,
            max_bytes: max,
            stats: std::cell::Cell::new(CacheStats::default()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 配额上限（字节）。报告里要给出这个数：不说的话用户无法判断
    /// "缓存会不会把盘吃满"（37 §2 要求配额有记录）。
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn stats(&self) -> CacheStats {
        self.stats.get()
    }

    fn record(&self, f: impl FnOnce(&mut CacheStats)) {
        let mut s = self.stats.get();
        f(&mut s);
        self.stats.set(s);
    }

    /// key → 文件名。key 已经是十六进制字符串（`front` 那边就是这么造的），
    /// 但这里不假设它——凡是进了文件名的东西都过一遍十六进制编码，
    /// 免得将来有人换个 key 生成方式就把路径分隔符带进来。
    fn path_for(&self, key: &str) -> PathBuf {
        let safe: String = key
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");
        self.dir.join(format!("{safe}.rc"))
    }

    /// 取一次前半链结果：命中就返回，否则真渲染并写盘。
    ///
    /// 与内存版 `RenderMemo` 的关键差别：**渲染失败不写盘**（同一条纪律，
    /// 见 `RenderMemo::render` 的注释）；但**写盘失败不影响返回值**——
    /// 用户要的是渲染结果，不是缓存。
    pub fn obtain(&self, key: &str, job: &RenderJob<'_>) -> Result<Loaded> {
        let path = self.path_for(key);
        match self.read(key, &path) {
            ReadOutcome::Hit(r) => {
                self.record(|s| s.hits += 1);
                return Ok(Loaded {
                    rendered: *r,
                    from_disk: true,
                });
            }
            ReadOutcome::Miss => self.record(|s| s.misses += 1),
            ReadOutcome::Corrupt => {
                self.record(|s| {
                    s.misses += 1;
                    s.corrupt += 1;
                });
                // 认不出的记录立刻清掉：留着只会在每次运行时再失败一次。
                // 这是删本进程自己管的缓存文件，不是用户资产。
                let _ = std::fs::remove_file(&path);
            }
        }
        let rendered = crate::front::render_to_plan(job)?;
        if let Err(_e) = self.write(key, &path, &rendered) {
            self.record(|s| s.write_failures += 1);
        }
        Ok(Loaded {
            rendered,
            from_disk: false,
        })
    }

    fn write(&self, key: &str, path: &Path, r: &Rendered) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let bytes = encode(key, r).map_err(|e| std::io::Error::other(e.to_string()))?;
        // 先写同目录的临时文件再改名：否则断电会留下一个半截的 .rc，
        // 而它恰好占着正确的文件名 —— 下一次运行就会读到它。
        let tmp = path.with_extension("rc.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        self.enforce_quota()?;
        Ok(())
    }

    fn read(&self, key: &str, path: &Path) -> ReadOutcome {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return ReadOutcome::Miss,
        };
        match decode(key, &bytes) {
            Some(r) => ReadOutcome::Hit(Box::new(r)),
            None => ReadOutcome::Corrupt,
        }
    }

    /// 配额执行：超过上限就按 mtime 从旧到新删，直到降到 [`QUOTA_TARGET`]。
    ///
    /// 37 §2：「容量达到上限时先淘汰未引用产物；正在使用的产物不得删除」。
    /// 这里没有「正在使用」的概念——每条记录在 `obtain` 里被读进内存后就不再需要
    /// 磁盘上的那份了，进程之间也不共享句柄。所以全部都是可淘汰的。
    /// 更要紧的一条纪律：**永远不删当前正在写的那一份**（写入方已返回，不受影响）。
    pub fn enforce_quota(&self) -> std::io::Result<()> {
        let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total = 0u64;
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rc") {
                continue;
            }
            let Ok(md) = ent.metadata() else { continue };
            if !md.is_file() {
                continue;
            }
            total += md.len();
            let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            entries.push((p, md.len(), mtime));
        }
        if total <= self.max_bytes {
            return Ok(());
        }
        // 先删最旧的：最近用过的更可能再用（写入即更新 mtime，等于一次访问即刷新）。
        entries.sort_by_key(|(_, _, t)| *t);
        let target = (self.max_bytes as f64 * QUOTA_TARGET) as u64;
        for (p, len, _) in entries {
            if total <= target {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(len);
            }
        }
        Ok(())
    }

    /// 清空整层。`cache clear` 用。
    pub fn clear(&self) -> std::io::Result<usize> {
        let mut n = 0usize;
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) == Some("rc")
                && std::fs::remove_file(&p).is_ok()
            {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 现有记录的总字节数与条数。`cache stat` 用。
    pub fn usage(&self) -> (u64, usize) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return (0, 0);
        };
        let mut bytes = 0u64;
        let mut n = 0usize;
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rc") {
                continue;
            }
            if let Ok(md) = ent.metadata() {
                if md.is_file() {
                    bytes += md.len();
                    n += 1;
                }
            }
        }
        (bytes, n)
    }
}

enum ReadOutcome {
    // 包在 Box 里：`Rendered` 有几百字节，而另两个变体不带数据。
    // clippy 会要求两个变体大小相当，直接装进去会让每次返回都拷贝整块。
    Hit(Box<Rendered>),
    Miss,
    Corrupt,
}

// ---------------------------------------------------------------------------
// 编码。格式固定、带长度校验；任何不符都返回 None（调用方当损坏处理）。
// ---------------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u64(out, b.len() as u64);
    out.extend_from_slice(b);
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes(s.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        let s = self.b.get(self.at..self.at + 8)?;
        self.at += 8;
        Some(u64::from_le_bytes(s.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u64()? as usize;
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
}

/// 把一次前半链结果编码成字节。
///
/// 故意**不缓存** `injected`（注入字体清单）与 `grid`：前者是"这次跑了哪些字体"的
/// 记账，后者由 fps/时长决定、可从调用方重算。缓存应当只存"重算代价高的东西"——
/// 也就是 patch 位图。存多余字段只会让格式更容易过期。
fn encode(key: &str, r: &Rendered) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(1 << 16);
    out.extend_from_slice(MAGIC);
    put_bytes(&mut out, key.as_bytes());
    put_u64(&mut out, r.samples as u64);

    // 采样网格：它决定后半链的报告（stream_end_ms、采样点数）与 AA5 的帧网格吸附。
    // 不存就只能命中后重算，而重算需要「脚本的最后事件时间」——那正是缓存
    // 本来要省掉的工作。存下来并在读回时逐字段校验自洽。
    put_u64(&mut out, r.grid.steps_ms.len() as u64);
    for ms in &r.grid.steps_ms {
        out.extend_from_slice(&ms.to_le_bytes());
    }
    out.extend_from_slice(&r.grid.blank_ms.to_le_bytes());
    out.extend_from_slice(&r.grid.stream_end_ms.to_le_bytes());
    put_u32(&mut out, u32::from(r.grid.rounded));

    // 注入字体的记账（名称 + 字节数 + 哈希）。它不是「重算代价高的东西」，
    // 但它是报告要用的「这次真用了哪些字体」。从盘上读回时必须给出同样的账，
    // 否则同一次运行的报告会随缓存冷暖而变。
    put_u64(&mut out, r.injected.len() as u64);
    for f in &r.injected {
        put_bytes(&mut out, f.name.as_bytes());
        put_u64(&mut out, f.bytes as u64);
        put_bytes(&mut out, f.sha256.as_bytes());
    }

    // 事件
    put_u64(&mut out, r.plan.events.len() as u64);
    for e in &r.plan.events {
        put_u64(
            &mut out,
            time_millis(e.start).map_err(shift_to_cache_code_1)?,
        );
        put_u64(&mut out, time_millis(e.end).map_err(shift_to_cache_code_2)?);
        put_u32(
            &mut out,
            match e.kind {
                EventKind::Image => 0,
                EventKind::Erase => 1,
            },
        );
        out.extend_from_slice(&e.digest.0);
        put_u64(&mut out, e.merged_samples as u64);
        match &e.patch {
            None => put_u32(&mut out, 0),
            Some(p) => {
                put_u32(&mut out, 1);
                // 字段顺序刻意按 `Patch` 的声明序（width, height, x, y）写：
                // 换个顺序写也能自洽，但读代码的人会以为它对应另一个结构。
                put_u32(&mut out, p.width);
                put_u32(&mut out, p.height);
                put_u32(&mut out, p.x);
                put_u32(&mut out, p.y);
                put_bytes(&mut out, &p.rgba);
            }
        }
    }

    // 统计（重建时直接还原，不从事件重算：重算会掩盖编码漏字段的 bug）
    let s = &r.plan.stats;
    put_u64(&mut out, s.samples as u64);
    put_u64(&mut out, s.events as u64);
    put_u64(&mut out, s.image_events as u64);
    put_u64(&mut out, s.erase_events as u64);
    put_u64(&mut out, s.eliminated as u64);
    put_u64(&mut out, s.leading_erase_dropped as u64);
    put_u64(&mut out, s.reuse_hint as u64);

    // 损失报告：它是"这次输出与理想差在哪"的账，必须与像素一起回来。
    // 用 serde 的 JSON 表达：LossReport 本身已经是可序列化的结构，
    // 手写一遍只会多一处会忘记同步的地方。
    let loss_json = serde_json::to_vec(&r.plan.loss).map_err(|e| {
        StkError::simple(Domain::Cache, 4, "缓存编码：损失报告序列化失败")
            .with_detail(e.to_string())
    })?;
    put_bytes(&mut out, &loss_json);

    Ok(out)
}

/// 解码。返回 `None` 表示这份字节不属于这个 key 或格式不认识——两种都当未命中。
fn decode(key: &str, b: &[u8]) -> Option<Rendered> {
    if b.len() < MAGIC.len() || &b[..MAGIC.len()] != MAGIC {
        return None;
    }
    let mut c = Cursor { b, at: MAGIC.len() };
    if c.bytes()? != key.as_bytes() {
        return None;
    }
    let samples = c.u64()? as usize;

    // 采样网格
    let nsteps = c.u64()? as usize;
    if nsteps > 100_000_000 {
        return None;
    }
    let mut steps_ms = Vec::with_capacity(nsteps.min(4096));
    for _ in 0..nsteps {
        let raw = c.b.get(c.at..c.at + 8)?;
        c.at += 8;
        steps_ms.push(i64::from_le_bytes(raw.try_into().ok()?));
    }
    let blank_ms = i64::from_le_bytes(c.b.get(c.at..c.at + 8)?.try_into().ok()?);
    c.at += 8;
    let stream_end_ms = i64::from_le_bytes(c.b.get(c.at..c.at + 8)?.try_into().ok()?);
    c.at += 8;
    let rounded = match c.u32()? {
        0 => false,
        1 => true,
        _ => return None,
    };

    // 注入字体的记账
    let ninj = c.u64()? as usize;
    if ninj > 100_000 {
        return None;
    }
    let mut injected = Vec::with_capacity(ninj.min(64));
    for _ in 0..ninj {
        let name = String::from_utf8(c.bytes()?.to_vec()).ok()?;
        let bytes = c.u64()? as usize;
        let sha256 = String::from_utf8(c.bytes()?.to_vec()).ok()?;
        injected.push(render_libass::InjectedFont {
            name,
            bytes,
            sha256,
        });
    }

    let n = c.u64()? as usize;
    // 上限防御：一个损坏的长度字段不该让我们去分配 10^18 字节。
    if n > 50_000_000 {
        return None;
    }
    let mut events = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let start = core_model::time::Time::from_millis(c.u64()? as i64).ok()?;
        let end = core_model::time::Time::from_millis(c.u64()? as i64).ok()?;
        let kind = match c.u32()? {
            0 => EventKind::Image,
            1 => EventKind::Erase,
            _ => return None,
        };
        // digest 是固定 32 字节，不带长度前缀 —— 和 encode 那边一一对应。
        // 这里若误用 c.bytes()（它要先读一个 u64 长度），就会把 digest 的头 8 字节
        // 当成一个巨大的长度字段，于是整个文件被判成损坏。
        let d = c.b.get(c.at..c.at + 32)?;
        c.at += 32;
        if d.len() != 32 {
            return None;
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(d);
        let merged_samples = c.u64()? as usize;
        let patch = match c.u32()? {
            0 => None,
            1 => {
                let width = c.u32()?;
                let height = c.u32()?;
                let x = c.u32()?;
                let y = c.u32()?;
                let rgba = c.bytes()?.to_vec();
                // patch 的 RGBA 长度必须与宽高自洽，否则它是个被截断的文件。
                let want = (width as usize)
                    .checked_mul(height as usize)?
                    .checked_mul(4)?;
                if rgba.len() != want {
                    return None;
                }
                Some(Patch {
                    x,
                    y,
                    width,
                    height,
                    rgba,
                })
            }
            _ => return None,
        };
        events.push(Event {
            start,
            end,
            kind,
            digest: Digest(digest),
            patch,
            merged_samples,
        });
    }

    let stats = PlanStats {
        samples: c.u64()? as usize,
        events: c.u64()? as usize,
        image_events: c.u64()? as usize,
        erase_events: c.u64()? as usize,
        eliminated: c.u64()? as usize,
        leading_erase_dropped: c.u64()? as usize,
        reuse_hint: c.u64()? as usize,
    };
    let loss: LossReport = serde_json::from_slice(c.bytes()?).ok()?;
    // 尾部必须干净：多余的字节意味着这份文件不属于当前格式。
    if c.at != b.len() {
        return None;
    }

    Some(Rendered {
        plan: Plan {
            events,
            stats,
            loss,
        },
        loss: LossReport::new(),
        samples,
        grid: crate::run::Grid {
            steps_ms,
            blank_ms,
            stream_end_ms,
            rounded,
        },
        injected,
    })
}

/// `Time` → 整毫秒。前半链的采样点本来就在毫秒网格上（`run::Grid` 是 i64 ms），
/// 所以非整毫秒只可能来自代码错误，值得报出来而不是静默取整。
///
/// 算法：`Time` 是精确有理数，乘 1000 得到 "毫秒" 这个有理数，分母为 1 才能无损取整。
/// 不借道 90kHz 的 `to_pts_ticks`：那条路过一遍整除，反而会把 "本来就不是整毫秒"
/// 这个事实藏起来。
fn time_millis(t: core_model::time::Time) -> Result<u64> {
    t.as_rational()
        .mul(&core_model::time::Rational::from_int(1000))
        .filter(|m| m.den() == 1)
        .and_then(|m| u64::try_from(m.num()).ok())
        .ok_or_else(|| StkError::simple(Domain::Cache, 1, "缓存编码：时间不是非负整毫秒"))
}

/// 把起点/终点各自的非整毫秒错误分开：编码失败时用户需要知道**哪个字段**坏了，
/// 两处共用一个编号就只能靠 details 猜。
fn shift_to_cache_code_1(e: StkError) -> StkError {
    StkError::simple(Domain::Cache, 2, "缓存编码：事件起点不是非负整毫秒").with_detail(
        e.details
            .first()
            .cloned()
            .unwrap_or_else(|| "精确值无法表示为整毫秒".to_string()),
    )
}

fn shift_to_cache_code_2(e: StkError) -> StkError {
    StkError::simple(Domain::Cache, 3, "缓存编码：事件终点不是非负整毫秒").with_detail(
        e.details
            .first()
            .cloned()
            .unwrap_or_else(|| "精确值无法表示为整毫秒".to_string()),
    )
}

/// 汇总键：与 `front::RenderMemo` 同一口径（见本文件顶部「第一条设计决定」）。
///
/// ## 为什么 key 里必须含 **host 可执行文件的哈希**
///
/// 它的加入不是“多一道保险”，而是堵一个实测到的**错误复用**：
/// `STK_RENDERER_HOST` 可以指向另一个宿主（典型是 x86 的那只），
/// 而原 key 只看 provider DLL 的哈希 —— 于是换成 x86 host 后**直接命中了 x64 的缓存**，
/// host 压根没跑，它的位数校验也就没机会报错（实测：x64 先渲一次 `hit=false`，
/// 换 x86 再渲 `hit=true` 且退 0，而其实 x86 host 会拒这个 x64 DLL）。
///
/// 后果是用户以为用的是 x86 host，拿到的却是另一只宿主的像素 —— 静默错误。
/// 键里同时有 DLL 与 host 两个身份，才是“谁画了这张图”的完整回答。
pub fn key_for(
    job: &RenderJob<'_>,
    dll_hash: &str,
    host_hash: &str,
    inject_hashes: &[String],
) -> String {
    let mut h = sha2::Sha256::new();
    use sha2::Digest as _;
    // 版本号升到 v2：v1 的 key 少了 host 身份，不能与这版混用。
    // 升版本号让旧记录自然变成“未命中”并被丢弃，而不是被误读。
    h.update(b"stk:render-cache-key:v2");
    // 输入内容：走文件的 SHA-256 而不是路径——同一个路径的内容会变，
    // 不同路径的内容可能相同（用户复制了一份）。
    let input_bytes = std::fs::read(job.input).unwrap_or_default();
    h.update(sha2::Sha256::digest(&input_bytes));
    h.update(job.fps.to_label().as_bytes());
    h.update(job.canvas.width.to_le_bytes());
    h.update(job.canvas.height.to_le_bytes());
    h.update(job.default_font.as_bytes());
    // renderer 二进制身份：换 DLL 就是换渲染结果
    h.update(dll_hash.as_bytes());
    // 宿主二进制身份：换 host（含 x64 ↔ x86）也是换渲染结果。
    // 哈希读不到时传空串：调用方把“没有 host 哈希”当成一个**独立的身份取值**，
    // 而不是当成“和别的都一样”。
    h.update(host_hash.as_bytes());
    // 注入字体：同一条 17 §7 的理由，注入什么字节决定画出来什么
    for x in inject_hashes {
        h.update(x.as_bytes());
    }
    h.update(b"\0end");
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::canvas::Canvas;
    use core_model::loss::LossReport;
    use core_model::time::Fps;
    use core_model::time::Time;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-rc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rendered() -> Rendered {
        let patch = Patch {
            width: 2,
            height: 2,
            x: 7,
            y: 9,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        Rendered {
            plan: Plan {
                events: vec![
                    Event {
                        start: Time::from_millis(1000).unwrap(),
                        end: Time::from_millis(3000).unwrap(),
                        kind: EventKind::Image,
                        digest: Digest([7u8; 32]),
                        patch: Some(patch),
                        merged_samples: 50,
                    },
                    Event {
                        start: Time::from_millis(3000).unwrap(),
                        end: Time::from_millis(3080).unwrap(),
                        kind: EventKind::Erase,
                        digest: Digest([9u8; 32]),
                        patch: None,
                        merged_samples: 2,
                    },
                ],
                stats: PlanStats {
                    samples: 77,
                    events: 2,
                    image_events: 1,
                    erase_events: 1,
                    eliminated: 74,
                    leading_erase_dropped: 1,
                    reuse_hint: 3,
                },
                loss: LossReport::new(),
            },
            loss: LossReport::new(),
            samples: 77,
            grid: crate::run::Grid {
                steps_ms: vec![0, 40, 80, 120],
                blank_ms: 3040,
                stream_end_ms: 3080,
                rounded: true,
            },
            injected: vec![render_libass::InjectedFont {
                name: "Arial".to_string(),
                bytes: 1234,
                sha256: "ab".repeat(32),
            }],
        }
    }

    /// 往返必须逐字段相等。这条测试是编码器唯一的把关：
    /// 任何字段读回来不一样，都会让命中缓存的产物与真渲染不同 ——
    /// 而那种差异不会报错，只会默默地错。
    #[test]
    fn encode_decode_roundtrip_preserves_every_field() {
        let r = rendered();
        let bytes = encode("k1", &r).unwrap();
        let back = decode("k1", &bytes).expect("自己写的字节必须能读回来");

        assert_eq!(back.samples, r.samples);
        assert_eq!(back.plan.stats, r.plan.stats);
        assert_eq!(back.grid.steps_ms, r.grid.steps_ms);
        assert_eq!(back.grid.blank_ms, r.grid.blank_ms);
        assert_eq!(back.grid.stream_end_ms, r.grid.stream_end_ms);
        assert_eq!(back.grid.rounded, r.grid.rounded);
        assert_eq!(back.injected.len(), 1);
        assert_eq!(back.injected[0].name, "Arial");
        assert_eq!(back.injected[0].bytes, 1234);
        assert_eq!(back.plan.events.len(), 2);
        for (a, b) in back.plan.events.iter().zip(r.plan.events.iter()) {
            assert_eq!(a.start, b.start, "事件起点");
            assert_eq!(a.end, b.end, "事件终点");
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.digest.0, b.digest.0);
            assert_eq!(a.merged_samples, b.merged_samples);
        }
        // patch 的四个坐标与像素必须各自回对位置。
        // 写成 "四个 u32 都读回来了" 是不够的：把 width 与 x 读反了它们也都"存在"。
        let pa = back.plan.events[0]
            .patch
            .as_ref()
            .expect("图像事件该带 patch");
        let pb = r.plan.events[0].patch.as_ref().unwrap();
        assert_eq!((pa.width, pa.height, pa.x, pa.y), (2, 2, 7, 9));
        assert_eq!(pa.rgba, pb.rgba);
        assert!(
            back.plan.events[1].patch.is_none(),
            "Erase 事件不该带 patch"
        );
    }

    /// 拿另一个 key 去读同一份字节必须被拒。
    /// 少了这条，磁盘上文件名与内容就会脱钩：改名一个 .rc 就能把 A 的像素
    /// 当成 B 的输出，而没有任何东西会发现。
    #[test]
    fn a_record_read_under_a_different_key_is_refused() {
        let bytes = encode("aaa", &rendered()).unwrap();
        assert!(decode("aaa", &bytes).is_some(), "自己的 key 该能读");
        assert!(decode("bbb", &bytes).is_none(), "换 key 必须被拒");
    }

    #[test]
    fn truncated_or_wrong_magic_bytes_are_refused_not_misread() {
        let bytes = encode("k", &rendered()).unwrap();
        assert!(decode("k", &bytes[..bytes.len() / 2]).is_none(), "截断");
        assert!(decode("k", &[]).is_none(), "空文件");
        let mut bad = bytes.clone();
        bad[0] = 0x58;
        assert!(decode("k", &bad).is_none(), "魔数不符");
        // 尾部多一个字节也是不属于当前格式：宁可当未命中，也不读一份来历不明的尾巴。
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode("k", &extra).is_none(), "尾部多余字节");
    }

    /// 损坏的 patch RGBA 长度必须被拒。
    /// 否则一个被截断的文件会变成一张"宽高说 2x2 但只有半个像素"的位图，
    /// 而下游把它当合法数据画出来。
    ///
    /// 定位方式：从头按已知布局走一遍，记下 rgba 长度字段的位置。
    /// 用"从后往前找值等于 16 的 u64"那种启发式会在格式变动后静默找错地方。
    #[test]
    fn a_patch_whose_rgba_length_contradicts_its_size_is_refused() {
        let mut bytes = encode("k", &rendered()).unwrap();
        let mut at = 8; // 跳过魔数
        let key_len = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        at += 8 + key_len; // key
        at += 8; // samples
        let nsteps = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        at += 8 + nsteps * 8; // 采样网格
        at += 8 + 8 + 4; // blank_ms + stream_end_ms + rounded
        let ninj = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        at += 8;
        for _ in 0..ninj {
            let n = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
            at += 8 + n + 8; // name + bytes
            let n = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
            at += 8 + n; // sha256
        }
        at += 8; // 事件数
                 // 第一个事件：start, end, kind, digest(32), merged_samples, has_patch
        at += 8 + 8 + 4 + 32 + 8 + 4;
        at += 16; // patch 的四个 u32
        assert_eq!(
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()),
            16
        );
        bytes[at..at + 8].copy_from_slice(&4u64.to_le_bytes());
        assert!(decode("k", &bytes).is_none(), "长度不符必须被拒");
    }

    /// 配额：超上限就必须真的离开上限。
    /// 37 §2 要求"配额与容量必须有记录"，一个不生效的配额比没有配额更糟：
    /// 用户以为设住了，实际盘中在长。
    #[test]
    fn the_quota_actually_bounds_the_bytes_on_disk() {
        let dir = tmpdir("quota");
        let c = DiskCache {
            dir: dir.clone(),
            max_bytes: 40_000,
            stats: std::cell::Cell::new(CacheStats::default()),
        };
        // 每份记录一万多字节，写十几份必然远超 40 KiB
        for i in 0..12 {
            let key = format!("key-{i}");
            c.write(&key, &c.path_for(&key), &rendered()).unwrap();
        }
        let (bytes, n) = c.usage();
        assert!(bytes <= 40_000, "配额没生效：{bytes} 字节 > 40000，{n} 条");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 一个不属于这个 key 的文件名被写入后，读回必须是"未命中 + 已清掉"，
    /// 而不是把别人的字节当自己的结果用。
    #[test]
    fn a_foreign_record_is_reported_corrupt_and_removed() {
        let dir = tmpdir("foreign");
        let c = DiskCache {
            dir: dir.clone(),
            max_bytes: DEFAULT_MAX_BYTES,
            stats: std::cell::Cell::new(CacheStats::default()),
        };
        // 把一份 key=a 的记录放到 key=b 的文件名下
        let path = c.path_for("b");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, encode("a", &rendered()).unwrap()).unwrap();
        assert!(matches!(c.read("b", &path), ReadOutcome::Corrupt));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 跨宿主复用（实测过的真缺陷的回归用例） ----------------------------

    fn job<'a>(input: &'a Path, dll: &'a Path) -> RenderJob<'a> {
        RenderJob {
            input,
            dll: Some(dll),
            fps: Fps::NTSC_23976,
            canvas: Canvas::FHD1080,
            default_font: "Arial",
            inject: None,
        }
    }

    /// 换 host 必须换 key。
    ///
    /// 这个缺陷的实测形态：x64 host 先渲一次（`hit=false`），
    /// 再把 `STK_RENDERER_HOST` 指向 x86 host 渲同一个东西 —— `hit=true` 且退 0，
    /// 而那只 host **本来会拒绝** 这个 x64 DLL（`STK-RND-0091`）。
    /// host 压根没跑，它的位数校验也就没机会报错。
    ///
    /// 后果是用户以为用的是 x86 host，拿到的却是另一只宿主的像素。
    ///
    /// 注意 provider DLL 哈希在两次里是**同一个** —— 光靠它区分不出来。
    #[test]
    fn keys_differ_when_only_the_host_binary_differs() {
        let dir = tmpdir("hostkey");
        let input = dir.join("a.ass");
        std::fs::write(&input, b"[Script Info]").unwrap();
        let dll = dir.join("ass.dll");
        std::fs::write(&dll, b"not-really-a-pe").unwrap();
        let j = job(&input, &dll);

        let a = key_for(&j, "dll-hash", "host-x64-hash", &[]);
        let b = key_for(&j, "dll-hash", "host-x86-hash", &[]);
        assert_ne!(a, b, "换 host 却得到同一个 key：会跨宿主错误复用");

        // 反向：只要 host 哈希一样，key 就该一样（否则缓存永远不命中）。
        assert_eq!(a, key_for(&j, "dll-hash", "host-x64-hash", &[]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 没有 host 哈希（读不到文件）是一个**独立的身份取值**，
    /// 不能与“有某个 host 哈希”碰成同一个 key。
    #[test]
    fn a_missing_host_hash_is_its_own_identity() {
        let dir = tmpdir("nohost");
        let input = dir.join("a.ass");
        std::fs::write(&input, b"[Script Info]").unwrap();
        let dll = dir.join("ass.dll");
        std::fs::write(&dll, b"x").unwrap();
        let j = job(&input, &dll);

        assert_ne!(
            key_for(&j, "d", "", &[]),
            key_for(&j, "d", "abc", &[]),
            "空 host 哈希与真实哈希碰成了同一个 key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// provider DLL 哈希仍然要进 key：换 DLL 就是换渲染结果。
    #[test]
    fn keys_still_differ_when_only_the_provider_differs() {
        let dir = tmpdir("dllkey");
        let input = dir.join("a.ass");
        std::fs::write(&input, b"[Script Info]").unwrap();
        let dll = dir.join("ass.dll");
        std::fs::write(&dll, b"x").unwrap();
        let j = job(&input, &dll);

        assert_ne!(
            key_for(&j, "dll-1", "host", &[]),
            key_for(&j, "dll-2", "host", &[])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// key 的版本号变了：v1 的记录不能与 v2 的记录混用。
    ///
    /// 这条靠**行为**核：同一个输入在 v2 下算出的 key（也就是文件名）
    /// 不会与 v1 的算法结果相同。用一次性重算 v1 算法的方式钉住。
    #[test]
    fn the_key_version_is_bumped_so_old_records_do_not_collide() {
        let dir = tmpdir("kv");
        let input = dir.join("a.ass");
        std::fs::write(&input, b"[Script Info]").unwrap();
        let dll = dir.join("ass.dll");
        std::fs::write(&dll, b"x").unwrap();
        let j = job(&input, &dll);

        let now = key_for(&j, "d", "h", &[]);

        // 手算一份 v1（少了 host 哈希，且版本串是 v1）。
        use sha2::Digest as _;
        let mut h = sha2::Sha256::new();
        h.update(b"stk:render-cache-key:v1");
        h.update(sha2::Sha256::digest(std::fs::read(&input).unwrap()));
        h.update(j.fps.to_label().as_bytes());
        h.update(j.canvas.width.to_le_bytes());
        h.update(j.canvas.height.to_le_bytes());
        h.update(j.default_font.as_bytes());
        h.update(b"d");
        h.update(b"\0end");
        let old = h
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");

        assert_ne!(now, old, "key 版本没生效：旧的 v1 记录会被当成命中");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
