//! AT-SEC-001（25 §验收、23 §健壮性）：恶意 PGS 长度/RLE 不崩溃、**不超额分配**。
//!
//! "不 panic"用模糊冒烟已经能覆盖；这条更难证明的是内存：一个声称 4 GiB payload 的
//! 头就分配 4 GiB，进程会先 OOM 再报错，等于没有防御。所以这里换一个能计数的手段：
//! 测试里装一个统计分配量的全局分配器，直接断言"最坏情况下也只分配了这么点"。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(l.size(), Ordering::SeqCst) + l.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // 只用 saturating：计数从进程一开始就跑，理论上不会负，但宁可钳住也不让测试
        // 因为一次簿记误差而 panic（panic 的测试会被误读成"防住了攻击"）
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let now = LIVE.fetch_add(new, Ordering::SeqCst) + new - l.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// 计数是全进程共享的，所以测量必须串行；而且**只算增量**：
/// 直接把 LIVE 归零会让之后任何一次 dealloc 簿记跑偏。
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn measure_peak<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let out = f();
    let peak = PEAK.load(Ordering::SeqCst).saturating_sub(base);
    (out, peak)
}

use codec_pgs::rle::decode as decode_rle;
use codec_pgs::segment::parse_stream;

/// 单个 segment 头：13 字节，payload 长度由外部声明。
fn seg_header(seg_type: u8, payload_len: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(seg_type);
    v.push(0x00); // version 00 00 00 00 00 00 01
    v.extend_from_slice(&[0u8; 5]);
    v.push(0x01);
    v.extend_from_slice(&payload_len.to_be_bytes());
    v
}

const HARD_CAP: usize = 64 << 20; // 64 MiB：恶意输入绝不该让我们分配到这个量级

#[test]
fn huge_declared_payload_does_not_allocate_it() {
    // 声称 4 GiB payload，但后面什么都没有
    let mut bytes = seg_header(codec_pgs::segment::SEG_PCS, u32::MAX);
    bytes.extend_from_slice(&[0u8; 16]);
    let (res, peak) = measure_peak(|| parse_stream(&bytes));
    assert!(res.is_err(), "截断的 payload 必须报错");
    assert!(peak < HARD_CAP, "为一个谎报 4 GiB 的头分配了 {} 字节", peak);
}

#[test]
fn many_declared_segments_are_capped() {
    // 每个 13 字节的 PCS 头，重复到远超 MAX_SEGMENTS：文件本身很小
    let one = seg_header(codec_pgs::segment::SEG_PCS, 0);
    let mut bytes = Vec::new();
    for _ in 0..(codec_pgs::segment::MAX_SEGMENTS + 10) {
        bytes.extend_from_slice(&one);
    }
    let (res, peak) = measure_peak(|| parse_stream(&bytes));
    assert!(res.is_err(), "超过段数上限必须拒绝");
    assert!(
        peak < bytes.len() * 4 + HARD_CAP,
        "拒绝之前分配了 {} 字节（文件才 {} 字节）",
        peak,
        bytes.len()
    );
}

#[test]
fn tiny_payload_claiming_a_giant_canvas_is_rejected() {
    // AT-SEC-001 的核心：2 字节 RLE 声称 8000x8000 画布
    let payload = [0x10u8, 0x00];
    let (res, peak) = measure_peak(|| decode_rle(&payload, 8000, 8000));
    assert!(res.is_err(), "尺寸超出支持上限要直接拒绝");
    assert!(peak < 4 << 20, "为 6400 万像素预分配了 {} 字节", peak);

    // 合法尺寸内也不许"极短 payload 谎报大画布"
    let small = [0x00u8, 0x00];
    let (res2, peak2) = measure_peak(|| decode_rle(&small, 2000, 2000));
    assert!(res2.is_err(), "payload 与画布比例不合理也要拒绝");
    assert!(peak2 < 4 << 20, "{} 字节", peak2);
}

#[test]
fn rle_run_that_never_terminates_is_stopped() {
    // 全是"跳过 N 像素"的指令，永远不写行尾：必须被像素预算卡住
    let mut payload = Vec::new();
    for _ in 0..4096 {
        payload.push(0xFF); // 颜色 255
        payload.push(64); // 连续 64 像素
    }
    let (res, peak) = measure_peak(|| decode_rle(&payload, 256, 256));
    assert!(res.is_err(), "超出 w*h 的游程必须报错而不是无限写");
    assert!(peak < HARD_CAP, "游程失控时分配了 {} 字节", peak);
}

#[test]
fn random_bytes_never_panic_and_stay_bounded() {
    // 小型确定性伪随机（与 cli 的 fuzz 门阶互补：这里量的是内存）
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut worst = 0usize;
    for i in 0..400 {
        let mut bytes = Vec::with_capacity(512);
        for _ in 0..256 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bytes.push((state >> 33) as u8);
        }
        let (r, peak) = measure_peak(|| {
            let s = parse_stream(&bytes);
            if let Ok(segs) = &s {
                for seg in segs {
                    let _ = codec_pgs::display_set::parse_ods_header(&seg.payload);
                    let _ = decode_rle(&seg.payload, 1920, 1080);
                }
            }
            s.is_err()
        });
        let _ = i;
        let _ = r;
        worst = worst.max(peak);
    }
    assert!(
        worst < 40 << 20,
        "随机字节最坏情况分配了 {} 字节：不合规的输入不该换来几十 MiB 的预分配",
        worst
    );
}
