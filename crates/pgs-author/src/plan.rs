//! Author 规划层（13 §4 必须项、13 §5 的"duplicate visual event elimination"）。
//!
//! 位置：渲染采样（`render-api::timeline`）与 `codec-pgs` Basic Encoder 之间。
//! codec-pgs 只认"完整画布上的 RGBA 对象 + pts"，它不知道哪些帧是重复的、
//! 哪些帧其实该是 erase display set —— 那是 author 的职责。
//!
//! 本层做的事（都是可证明的）：
//! 1. **可见内容指纹**：对 bbox 内的 RGBA（含 alpha）算 SHA-256。全透明帧没有 bbox，
//!    指纹是同一个"空"值（09 §3：alpha=0 的残影不算内容，绝不能因 RGB 脏值不同而拆成两个事件）。
//! 2. **重复视觉事件消除**：连续同指纹的采样并成一个事件（一个显示集），
//!    时长由下一个不同事件的起点或 `stream_end` 决定。
//! 3. **空白段 → erase**：中间的空白段成一个 erase 事件；开头的空白段直接丢弃
//!    （屏幕上还没有东西可擦），并计入 LossReport。
//! 4. **局部位图**：每个事件只交出 bbox 裁剪结果，x/y 仍是完整画布坐标（09 §3）。
//!
//! 明确**不做**（没有依据就不声称）：object reuse / palette animation / epoch planning /
//! coded object buffer / acquisition point / 主动丢帧或降码率（13 §5、13 §5.1）。
//! 同指纹但被别的内容隔开的重复只统计成 `reuse_hint`，供人决策，不改流。

use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use core_model::time::{Fps, Rational, Time};
use image_core::{Frame, Patch};
use sha2::{Digest as Sha2Digest, Sha256};

/// 内容指纹：SHA-256，覆盖 bbox 位置 + 局部位图字节。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// 空内容（全透明帧）的指纹：不看 RGB 残影。
pub fn empty_digest() -> Digest {
    Digest(Sha256::digest(b"stk:pgs:empty").into())
}

fn digest_of(patch: &Patch) -> Digest {
    let mut h = Sha256::new();
    // 位置也进指纹：同一张图挪了位置就是不同的视觉事件
    h.update(b"stk:pgs:patch");
    h.update(u32::to_be_bytes(patch.x));
    h.update(u32::to_be_bytes(patch.y));
    h.update(u32::to_be_bytes(patch.width));
    h.update(u32::to_be_bytes(patch.height));
    h.update(&patch.rgba);
    Digest(h.finalize().into())
}

/// 一次渲染采样：某个时间点的完整画布位图。
#[derive(Clone, Debug)]
pub struct Sample {
    pub at: Time,
    pub frame: Frame,
}

impl Sample {
    pub fn new(at: Time, frame: Frame) -> Sample {
        Sample { at, frame }
    }

    /// 可见内容指纹（没有 bbox 就是空）。
    pub fn digest(&self) -> Result<Digest> {
        match self.frame.crop()? {
            Some(p) => Ok(digest_of(&p)),
            None => Ok(empty_digest()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// 有可见内容：一个 composition object
    Image,
    /// 空白段：erase display set
    Erase,
}

/// 合并后的一个视觉事件。
#[derive(Clone, Debug)]
pub struct Event {
    pub start: Time,
    pub end: Time,
    pub kind: EventKind,
    pub digest: Digest,
    /// 事件起点那一帧的局部位图（Erase 时为 None）
    pub patch: Option<Patch>,
    /// 被这个事件吸收的采样点数（>=1）
    pub merged_samples: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanStats {
    pub samples: usize,
    pub events: usize,
    pub image_events: usize,
    pub erase_events: usize,
    /// 被"重复视觉事件消除"吸收掉的采样点数 = samples - events
    pub eliminated: usize,
    pub leading_erase_dropped: usize,
    /// 同指纹但被别的内容隔开的次数（13 §5 object reuse 的提示，不改变输出）
    pub reuse_hint: usize,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub events: Vec<Event>,
    pub stats: PlanStats,
    pub loss: LossReport,
}

fn err(code: u16, title: &str) -> StkError {
    StkError::simple(Domain::Pgs, code, title)
}

/// 规划选项。`stream_end` 必填：最后一个事件的结束时间只能由调用方给，
/// author 不猜（猜出来的时长会变成未经授权的改动）。
#[derive(Clone, Copy, Debug)]
pub struct AuthorOptions {
    pub stream_end: Time,
}

/// 把采样序列规划成显示集事件。采样时间必须严格递增。
pub fn plan(samples: &[Sample], opt: &AuthorOptions) -> Result<Plan> {
    if samples.is_empty() {
        return Err(err(110, "没有任何渲染采样"));
    }
    for i in 1..samples.len() {
        if samples[i].at <= samples[i - 1].at {
            return Err(err(111, "渲染采样时间不是严格递增")
                .with_detail(format!("samples[{i}] 的 PTS 不晚于前一个点"))
                .with_action(
                    "采样必须来自 render-api 的有序时间线计划；重复或倒序点会产出非法 PTS",
                ));
        }
    }
    let last = samples[samples.len() - 1].at;
    if opt.stream_end <= last {
        return Err(
            err(112, "stream_end 必须晚于最后一个采样点").with_detail(format!(
                "最后采样 PTS {} tick，stream_end PTS {} tick",
                last.to_pts_ticks().unwrap_or(i64::MIN),
                opt.stream_end.to_pts_ticks().unwrap_or(i64::MAX)
            )),
        );
    }

    // 1) 逐点指纹
    let mut digests: Vec<Digest> = Vec::with_capacity(samples.len());
    for s in samples {
        digests.push(s.digest()?);
    }

    // 2) 连续同指纹成段
    let mut runs: Vec<(usize, usize, Digest)> = Vec::new();
    let mut start = 0usize;
    for i in 1..samples.len() {
        if digests[i] != digests[start] {
            runs.push((start, i, digests[start]));
            start = i;
        }
    }
    runs.push((start, samples.len(), digests[start]));

    let mut loss = LossReport::new();
    let mut stats = PlanStats {
        samples: samples.len(),
        ..Default::default()
    };
    let mut events: Vec<Event> = Vec::with_capacity(runs.len());
    let mut seen: Vec<Digest> = Vec::new();

    for (ri, (from, to, dg)) in runs.iter().enumerate() {
        let is_empty = *dg == empty_digest();
        let ev_end = runs
            .get(ri + 1)
            .map(|(n, _, _)| samples[*n].at)
            .unwrap_or(opt.stream_end);
        if is_empty {
            if events.is_empty() {
                // 开头空白：没有内容可擦，丢掉
                stats.leading_erase_dropped += 1;
                continue;
            }
            // 连续空白段只留一个 erase（前一个 erase 已经把屏幕擦干净了）
            if let Some(prev) = events.last() {
                if prev.kind == EventKind::Erase {
                    stats.eliminated = stats.eliminated.saturating_add(to - from);
                    continue;
                }
            }
            stats.erase_events += 1;
            events.push(Event {
                start: samples[*from].at,
                end: ev_end,
                kind: EventKind::Erase,
                digest: *dg,
                patch: None,
                merged_samples: to - from,
            });
            continue;
        }
        if let Some(prev) = seen.last() {
            if prev == dg {
                // 紧邻重复已经在 run 合并里处理过；这里只可能因为 erase 段隔开
                stats.reuse_hint += 1;
            }
        }
        let patch = samples[*from]
            .frame
            .crop()?
            .ok_or_else(|| err(113, "非空指纹却没有 bbox（内部不一致）"))?;
        stats.image_events += 1;
        events.push(Event {
            start: samples[*from].at,
            end: ev_end,
            kind: EventKind::Image,
            digest: *dg,
            patch: Some(patch),
            merged_samples: to - from,
        });
        seen.push(*dg);
    }

    stats.events = events.len();
    stats.eliminated = stats
        .eliminated
        .saturating_add(samples.len().saturating_sub(runs.len()));
    if stats.eliminated > 0 {
        loss.dropped(&format!(
            "重复视觉事件消除：{} 个采样点并入相邻事件（13 §5）",
            stats.eliminated
        ));
    }
    if stats.leading_erase_dropped > 0 {
        loss.dropped(&format!(
            "开头 {} 段空白没有可擦除内容，丢弃 erase 显示集",
            stats.leading_erase_dropped
        ));
    }
    if stats.reuse_hint > 0 {
        loss.user_attention(&format!(
            "{} 次内容重复但被空白段隔开：Advanced Author 的 object reuse 未实现（13 §5），这里只提示不改流",
            stats.reuse_hint
        ));
    }
    if events.is_empty() {
        loss.dropped("整条流都是空白：不产出任何显示集");
    }
    Ok(Plan {
        events,
        stats,
        loss,
    })
}

impl Plan {
    /// 把每个显示集的**起点**吸附到帧网格上（AA5：让"严格蓝光兼容"这条声明真的可达）。
    ///
    /// 只在**一帧周期是整数 tick** 时允许。23.976 的一帧是 3753.75 tick，
    /// 若允许取整成 3754 再吸，产出的是一条谁都不认的**假网格** —— validator 的
    /// `pts % ticks` 还会跟着自洽地通过，等于用断言替错误前提背书。所以这里拒绝。
    ///
    /// 方向是 floor（提前到帧边界）：内容只会更早出现、不会更晚消失，也不会把最后一个
    /// 事件推过 `stream_end`。相邻事件 floor 到同一格时把后面那个推到下一格 ——
    /// 事件数与顺序都不许变（PTS 非单调递增会被 `STK-PGS-0090` 拦，那条路不该走）。
    ///
    /// 注意一个真实的相互作用：网格原点是 0，所以起点落在第一格内的事件会被吸到 PTS 0，
    /// 那低于编码器要求的 decode lead，`codec_pgs::encoder` 会响亮地拒绝
    /// （`STK-PGS-0098`）而不是偷偷挪网格原点 —— 由用户决定换 FPS 还是减小吸附。
    pub fn snap_starts_to_grid(&mut self, fps: Fps) -> Result<usize> {
        let period = Rational::from_int(90_000)
            .div(&fps.as_rational())
            .ok_or_else(|| err(119, "帧网格的 tick 周期算不出来"))?;
        if period.den() != 1 || period.num() <= 0 {
            return Err(err(119, "这个 FPS 的一帧不是整数 tick，拒绝吸附")
                .with_detail(format!("一帧 = {period} × 90 kHz tick，分母不是 1"))
                .with_action(
                    "只有整数 tick 的网格（24/25/30/50/60 等）才能声称帧边界；\
                     23.976/29.97 请保持不吸附（AA5、10 §6）",
                ));
        }
        let ticks = period.num();
        let mut moved = 0usize;
        let mut prev: Option<i64> = None;
        for ev in &mut self.events {
            let raw = ev.start.to_pts_ticks()?;
            let mut snapped = (raw / ticks) * ticks;
            if let Some(p) = prev {
                if snapped <= p {
                    snapped = p + ticks;
                }
            }
            if snapped != raw {
                ev.start = Time::from_pts_ticks(snapped)?;
                moved += 1;
            }
            prev = Some(snapped);
        }
        Ok(moved)
    }

    /// 交给 Basic Encoder 的帧序列。每个事件一帧；erase 事件是 `Frame::erase`。
    pub fn codec_frames(&self) -> Result<Vec<codec_pgs::encoder::Frame>> {
        let mut out = Vec::with_capacity(self.events.len());
        for (i, ev) in self.events.iter().enumerate() {
            let pts = ev
                .start
                .to_pts_ticks()
                .map_err(|e| e.with_detail(format!("events[{i}]")))?;
            match (&ev.patch, ev.kind) {
                (None, EventKind::Erase) => out.push(codec_pgs::encoder::Frame::erase(pts)),
                (Some(p), EventKind::Image) => {
                    let obj = codec_pgs::encoder::FrameObject {
                        rgba: p.rgba.clone(),
                        width: u16::try_from(p.width)
                            .map_err(|_| err(114, "对象宽度超出 PGS 16 位字段"))?,
                        height: u16::try_from(p.height)
                            .map_err(|_| err(115, "对象高度超出 PGS 16 位字段"))?,
                        x: u16::try_from(p.x).map_err(|_| err(116, "对象 x 超出 PGS 16 位字段"))?,
                        y: u16::try_from(p.y).map_err(|_| err(117, "对象 y 超出 PGS 16 位字段"))?,
                        forced: false,
                    };
                    out.push(codec_pgs::encoder::Frame::new(pts, vec![obj]));
                }
                _ => {
                    return Err(
                        err(118, "事件 kind 与 patch 不一致").with_detail(format!("events[{i}]"))
                    );
                }
            }
        }
        Ok(out)
    }
}

/// 规划 + 编码 + 自检（13 §6 的第 1-4、6 层）。
struct ActiveRun {
    start: Time,
    digest: Digest,
    patch: Option<Patch>,
    samples: usize,
}

/// 增量规划器。每次只保留当前 run 的 tight patch，完整 RGBA 帧可立即释放。
pub struct PlanBuilder {
    options: AuthorOptions,
    active: Option<ActiveRun>,
    last_at: Option<Time>,
    sample_count: usize,
    run_count: usize,
    events: Vec<Event>,
    seen: Vec<Digest>,
    stats: PlanStats,
    loss: LossReport,
}

impl PlanBuilder {
    pub fn new(options: AuthorOptions) -> PlanBuilder {
        PlanBuilder {
            options,
            active: None,
            last_at: None,
            sample_count: 0,
            run_count: 0,
            events: Vec::new(),
            seen: Vec::new(),
            stats: PlanStats::default(),
            loss: LossReport::new(),
        }
    }

    pub fn push(&mut self, sample: Sample) -> Result<()> {
        if let Some(prev) = self.last_at {
            if sample.at <= prev {
                return Err(err(111, "渲染采样时间不是严格递增")
                    .with_detail(format!("采样 {} 不晚于前一个点", self.sample_count))
                    .with_action(
                        "采样必须来自 render-api 的有序时间线计划；重复或倒序点会产出非法 PTS",
                    ));
            }
        }

        let patch = sample.frame.crop()?;
        let digest = patch.as_ref().map(digest_of).unwrap_or_else(empty_digest);
        self.sample_count += 1;

        if let Some(active) = self.active.as_mut() {
            if active.digest == digest {
                active.samples += 1;
            } else {
                let next = ActiveRun {
                    start: sample.at,
                    digest,
                    patch,
                    samples: 1,
                };
                let current = self.active.replace(next).expect("active run was checked");
                self.finish_run(current, sample.at)?;
            }
        } else {
            self.active = Some(ActiveRun {
                start: sample.at,
                digest,
                patch,
                samples: 1,
            });
        }
        self.last_at = Some(sample.at);
        Ok(())
    }

    pub fn finish(mut self) -> Result<Plan> {
        let last = self.last_at.ok_or_else(|| err(110, "没有任何渲染采样"))?;
        if self.options.stream_end <= last {
            return Err(
                err(112, "stream_end 必须晚于最后一个采样点").with_detail(format!(
                    "最后采样 PTS {} tick，stream_end PTS {} tick",
                    last.to_pts_ticks().unwrap_or(i64::MIN),
                    self.options.stream_end.to_pts_ticks().unwrap_or(i64::MAX)
                )),
            );
        }
        let final_run = self.active.take().expect("a sample created an active run");
        self.finish_run(final_run, self.options.stream_end)?;
        self.stats.samples = self.sample_count;
        self.stats.events = self.events.len();
        self.stats.eliminated = self
            .stats
            .eliminated
            .saturating_add(self.sample_count.saturating_sub(self.run_count));

        if self.stats.eliminated > 0 {
            self.loss.dropped(&format!(
                "重复视觉事件消除：{} 个采样点并入相邻事件（13 §5）",
                self.stats.eliminated
            ));
        }
        if self.stats.leading_erase_dropped > 0 {
            self.loss.dropped(&format!(
                "开头 {} 段空白没有可擦除内容，丢弃 erase 显示集",
                self.stats.leading_erase_dropped
            ));
        }
        if self.stats.reuse_hint > 0 {
            self.loss.user_attention(&format!(
                "{} 次内容重复但被空白段隔开：Advanced Author 的 object reuse 未实现（13 §5），这里只提示不改流",
                self.stats.reuse_hint
            ));
        }
        if self.events.is_empty() {
            self.loss.dropped("整条流都是空白：不产出任何显示集");
        }
        Ok(Plan {
            events: self.events,
            stats: self.stats,
            loss: self.loss,
        })
    }

    fn finish_run(&mut self, run: ActiveRun, end: Time) -> Result<()> {
        self.run_count += 1;
        if run.digest == empty_digest() {
            if self.events.is_empty() {
                self.stats.leading_erase_dropped += 1;
                return Ok(());
            }
            if self
                .events
                .last()
                .is_some_and(|event| event.kind == EventKind::Erase)
            {
                self.stats.eliminated = self.stats.eliminated.saturating_add(run.samples);
                return Ok(());
            }
            self.stats.erase_events += 1;
            self.events.push(Event {
                start: run.start,
                end,
                kind: EventKind::Erase,
                digest: run.digest,
                patch: None,
                merged_samples: run.samples,
            });
            return Ok(());
        }

        if self
            .seen
            .last()
            .is_some_and(|previous| *previous == run.digest)
        {
            self.stats.reuse_hint += 1;
        }
        let patch = run
            .patch
            .ok_or_else(|| err(113, "非空指纹却没有 bbox（内部不一致）"))?;
        self.stats.image_events += 1;
        self.events.push(Event {
            start: run.start,
            end,
            kind: EventKind::Image,
            digest: run.digest,
            patch: Some(patch),
            merged_samples: run.samples,
        });
        self.seen.push(run.digest);
        Ok(())
    }
}

pub fn author(
    samples: &[Sample],
    aopt: &AuthorOptions,
    eopt: &codec_pgs::encoder::EncodeOptions,
    want: &validator::ProfileWant,
    grid: Option<core_model::time::Fps>,
    bitrate: Option<&validator::BitrateArgs>,
) -> Result<Authored> {
    author_from_plan(plan(samples, aopt)?, eopt, want, grid, bitrate)
}

/// 已经规划好的那份"变化检测结果"直接进入编码/自检。
///
/// 为什么要单独开这条口子：14 §ASS→SUP 明确写着"若同时输出 BDN+SUP，共享 render/cache"。
/// 共享的就是这一步的输出（[`Plan`]），而 `author` 的签名要求传采样——传采样就等于
/// 每个输出各渲染一遍。调用方（CLI 的批量执行器）拿一份 Plan 分给两种输出，
/// 前半链就只跑一次；语义与 `author` 完全一致，因为它只是同一函数体的前半段被拆出来。
pub fn author_from_plan(
    mut plan: Plan,
    eopt: &codec_pgs::encoder::EncodeOptions,
    want: &validator::ProfileWant,
    grid: Option<core_model::time::Fps>,
    bitrate: Option<&validator::BitrateArgs>,
) -> Result<Authored> {
    let frames = plan.codec_frames()?;
    let (segments, encode_report) = codec_pgs::encoder::encode(&frames, eopt)?;
    // 量化过的颜色必须冒到 LossReport 顶层（08 §4）：计划阶段的损耗记在 plan.loss，
    // 编码阶段的损耗记在这里，用户在 CLI envelope 里看到的才是完整的一条账。
    if encode_report.quantized_frames > 0 {
        plan.loss.approximated(&format!(
            "Advanced Author 量化：{} 帧把 {} 个可分辨颜色合并进 255 色预算，单通道最大偏差 {}",
            encode_report.quantized_frames,
            encode_report.merged_colors,
            encode_report.worst_color_error
        ));
    }
    let bytes = codec_pgs::segment::write_stream(&segments)?;
    let validation = validator::pgs(&segments, want, grid, bitrate);
    let bitrate_measure = bitrate.map(|args| validator::peak_bitrate(&segments, args));
    Ok(Authored {
        plan,
        segments,
        bytes,
        encode_report,
        validation,
        bitrate: bitrate_measure,
    })
}

/// `author` 的产物：字节 + 全部中间证据（22 §诊断要求"一次运行能自证到哪一步"）。
pub struct Authored {
    pub plan: Plan,
    pub segments: Vec<codec_pgs::segment::Segment>,
    pub bytes: Vec<u8>,
    pub encode_report: codec_pgs::encoder::EncodeReport,
    pub validation: validator::ValidationReport,
    pub bitrate: Option<validator::BitrateMeasurement>,
}

impl Authored {
    /// 校验有 error/fatal 就把流拦下来，不交付。
    pub fn ensure_valid(&self) -> Result<()> {
        validator::to_error(&self.validation, "PGS author 输出")
    }

    pub fn summary(&self) -> String {
        format!(
            "事件 {}（图 {} / erase {}），显示集 {}，ODS 段 {}，采样消除 {}，校验 {}",
            self.plan.stats.events,
            self.plan.stats.image_events,
            self.plan.stats.erase_events,
            self.encode_report.display_sets,
            self.encode_report.ods_segments,
            self.plan.stats.eliminated,
            if self.validation.no_blocking_findings() {
                "无 error/fatal"
            } else {
                "有 error/fatal"
            }
        )
    }
}
