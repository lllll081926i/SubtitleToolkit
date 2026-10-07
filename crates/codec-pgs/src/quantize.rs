//! 调色板预算（13 §4）与颜色量化（13 §5 "image quantization quality vs decode cost"）。
//!
//! 两档语义，不许混：
//! - [`ColorPolicy::Exact`]（Basic Encoder）：一条颜色都不合并，超过预算就报错。
//!   真实 libass 的抗锯齿输出经常超过预算（ISSUES Z3），所以 Exact 只证明"结构正确"。
//! - [`ColorPolicy::MedianCut`]（Advanced Author）：把可分辨颜色合并进预算，
//!   **误差必须被度量出来**交给调用方写进 LossReport——静默失真比失败更糟。
//!
//! 量化在 `(Y, Cr, Cb, T)` 空间做，因为那正是 PGS 调色板 5 字节条目存的东西：
//! 在 RGB 空间算得再准，落到条目上还是会漂。`T`（不透明度）既参与切分也参与距离——
//! 字幕的边缘靠的就是这条 alpha 阶梯，把它二值化等于把抗锯齿抹掉。

use std::collections::HashMap;

use core_model::error::{Domain, Result, StkError};

use crate::palette::{ColorMatrix, PaletteEntry};

/// PGS 调色板最多 256 条，索引 0 固定为透明，所以可用颜色预算是 255（13 §4）。
pub const COLOR_BUDGET: usize = 255;
pub const TRANSPARENT_INDEX: u8 = 0;

/// 编码后的颜色键：`[Y, Cr, Cb, T]`。
pub type Key = [u8; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorPolicy {
    /// Basic：不合并；`distinct > COLOR_BUDGET` 直接失败。
    #[default]
    Exact,
    /// Advanced：median-cut 合并到 `max_colors` 条以内（1..=255）。
    MedianCut { max_colors: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantStats {
    /// 帧内出现过的可分辨颜色数（透明像素不计）
    pub distinct: usize,
    /// 最终占用的调色板条数
    pub used: usize,
    /// 被合并掉的颜色数
    pub merged: usize,
    /// 单通道最大绝对偏差；0 表示无损
    pub worst: u32,
    /// 按像素数加权的均方偏差
    pub mean_sq: u64,
    pub pixels: u64,
}

/// RGBA 像素 -> 调色板键。全透明像素先归一化，免得同一语义的颜色占多个槽位。
pub fn key_of(rgba: [u8; 4], matrix: ColorMatrix) -> Key {
    if rgba[3] == 0 {
        return [0, 0x80, 0x80, 0];
    }
    let e = PaletteEntry::from_rgba(rgba, TRANSPARENT_INDEX, matrix);
    [e.y, e.cr, e.cb, e.alpha]
}

/// 统计若干位图里出现过的颜色及像素数。
pub fn histogram<'a>(
    bitmaps: impl Iterator<Item = &'a [[u8; 4]]>,
    matrix: ColorMatrix,
) -> HashMap<Key, u64> {
    let mut hist: HashMap<Key, u64> = HashMap::new();
    for px in bitmaps.flatten() {
        *hist.entry(key_of(*px, matrix)).or_insert(0) += 1;
    }
    hist
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorTable {
    /// 第 0 条固定是透明槽位
    pub entries: Vec<PaletteEntry>,
    map: HashMap<Key, u8>,
    /// `Some` 只出现在真的合并过颜色时
    pub stats: Option<QuantStats>,
}

impl ColorTable {
    /// 键 -> 调色板索引。
    ///
    /// 表通常由同一批像素建出；但把它复用给别的像素时，查不到也得给出**合法**索引，
    /// 所以这里按最近代表色现算，既不 panic 也不悄悄退成透明。
    pub fn index_of(&self, key: Key) -> u8 {
        if let Some(i) = self.map.get(&key) {
            return *i;
        }
        if key[3] == 0 {
            return TRANSPARENT_INDEX;
        }
        let mut best = TRANSPARENT_INDEX;
        let mut best_d = u64::MAX;
        for e in self.entries.iter().skip(1) {
            let d = dist2(key, [e.y, e.cr, e.cb, e.alpha]);
            if d < best_d {
                best_d = d;
                best = e.index;
            }
        }
        best
    }
}

fn dist2(a: Key, b: Key) -> u64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = i64::from(*x) - i64::from(y);
            (d * d) as u64
        })
        .sum()
}

fn max_channel_delta(a: Key, b: Key) -> u32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| u32::from(x.abs_diff(y)))
        .max()
        .unwrap_or(0)
}

/// 由直方图建索引表。`Err` 只有一种：Exact 档超预算。
pub fn build_table(hist: &HashMap<Key, u64>, policy: ColorPolicy) -> Result<ColorTable> {
    let budget = match policy {
        ColorPolicy::Exact => COLOR_BUDGET,
        ColorPolicy::MedianCut { max_colors } => {
            if max_colors == 0 {
                return Err(StkError::simple(Domain::Pgs, 103, "max_colors 至少要是 1"));
            }
            (max_colors as usize).min(COLOR_BUDGET)
        }
    };
    // 排序保证：同样的输入永远得到同样的调色板与同样的索引（golden 与像素 diff 的前提）。
    // T==0 的颜色不进表：它们恒等于索引 0（13 §4 的透明槽位约定）。
    let mut distinct: Vec<Key> = hist.keys().copied().filter(|k| k[3] != 0).collect();
    distinct.sort_unstable();
    let weights: Vec<u64> = distinct.iter().map(|k| hist[k]).collect();
    let pixels: u64 = weights.iter().sum();

    if distinct.len() <= budget {
        let mut entries = vec![PaletteEntry::transparent(TRANSPARENT_INDEX)];
        let mut map = HashMap::new();
        for (i, k) in distinct.iter().enumerate() {
            entries.push(entry_of(*k, (i + 1) as u8));
            map.insert(*k, (i + 1) as u8);
        }
        return Ok(ColorTable {
            entries,
            map,
            stats: None,
        });
    }

    let ColorPolicy::MedianCut { .. } = policy else {
        return Err(StkError::simple(
            Domain::Pgs,
            95,
            "超过 255 个可分辨颜色，Basic Encoder 不做抖动或近似合并",
        )
        .with_detail(format!(
            "本帧有 {} 个可分辨颜色，预算 {budget}",
            distinct.len()
        ))
        .with_action("需要 Advanced Author 的量化/抖动策略（13 §5），或降低源图色深"));
    };

    let reps = median_cut(&distinct, &weights, budget);
    let mut entries = vec![PaletteEntry::transparent(TRANSPARENT_INDEX)];
    for (i, r) in reps.iter().enumerate() {
        entries.push(entry_of(*r, (i + 1) as u8));
    }
    let mut map = HashMap::with_capacity(distinct.len());
    let mut worst = 0u32;
    let mut sum_sq = 0u64;
    for (k, w) in distinct.iter().zip(weights.iter()) {
        let mut best = 0usize;
        let mut best_d = u64::MAX;
        for (ri, r) in reps.iter().enumerate() {
            let d = dist2(*k, *r);
            if d < best_d {
                best_d = d;
                best = ri;
            }
        }
        map.insert(*k, (best + 1) as u8);
        worst = worst.max(max_channel_delta(*k, reps[best]));
        sum_sq += best_d * w;
    }
    Ok(ColorTable {
        entries,
        map,
        stats: Some(QuantStats {
            distinct: distinct.len(),
            used: reps.len(),
            merged: distinct.len() - reps.len(),
            worst,
            mean_sq: sum_sq.checked_div(pixels).unwrap_or(0),
            pixels,
        }),
    })
}

fn entry_of(k: Key, index: u8) -> PaletteEntry {
    PaletteEntry {
        index,
        y: k[0],
        cr: k[1],
        cb: k[2],
        alpha: k[3],
    }
}

/// median-cut：每次在"跨度最大的桶"沿"它最宽的通道"按像素数取中位数切一刀。
fn median_cut(distinct: &[Key], weights: &[u64], budget: usize) -> Vec<Key> {
    let mut buckets: Vec<Vec<usize>> = vec![(0..distinct.len()).collect()];
    while buckets.len() < budget {
        let mut pick: Option<(usize, usize, u32)> = None;
        for (bi, b) in buckets.iter().enumerate() {
            let Some((dim, range)) = widest(distinct, b) else {
                continue;
            };
            // 严格大于：同跨度时保留先出现的桶，切分顺序才确定
            if pick.is_none_or(|(_, _, r)| range > r) {
                pick = Some((bi, dim, range));
            }
        }
        let Some((bi, dim, _)) = pick else {
            break; // 所有桶都只剩单色了，再切也不增加条目数
        };
        let mut items = buckets[bi].clone();
        items.sort_unstable_by(|x, y| {
            distinct[*x][dim]
                .cmp(&distinct[*y][dim])
                .then_with(|| distinct[*x].cmp(&distinct[*y]))
                .then_with(|| x.cmp(y))
        });
        let total: u64 = items.iter().map(|i| weights[*i]).sum();
        let mut acc = 0u64;
        let mut cut = 0usize;
        for (i, idx) in items.iter().enumerate() {
            acc += weights[*idx];
            cut = i + 1;
            if acc * 2 >= total {
                break;
            }
        }
        let cut = cut.clamp(1, items.len() - 1);
        let right = items.split_off(cut);
        buckets[bi] = items;
        buckets.push(right);
    }
    let mut reps: Vec<Key> = buckets
        .iter()
        .map(|b| weighted_mean(distinct, weights, b))
        .collect();
    reps.sort_unstable();
    reps.dedup();
    reps
}

/// 桶内四个通道各自的 (最小值, 最大值)。
fn channel_bounds(distinct: &[Key], bucket: &[usize]) -> Option<(Key, Key)> {
    let mut it = bucket.iter();
    let first = distinct[*it.next()?];
    let (mut lo, mut hi) = (first, first);
    for i in it {
        let k = distinct[*i];
        for (slot, v) in lo.iter_mut().zip(k) {
            *slot = (*slot).min(v);
        }
        for (slot, v) in hi.iter_mut().zip(k) {
            *slot = (*slot).max(v);
        }
    }
    Some((lo, hi))
}

/// 桶内跨度最大的通道；返回 `None` 表示这个桶已经是单色，切了也没用。
fn widest(distinct: &[Key], bucket: &[usize]) -> Option<(usize, u32)> {
    let (lo, hi) = channel_bounds(distinct, bucket)?;
    let mut best: Option<(usize, u32)> = None;
    for (dim, (l, h)) in lo.into_iter().zip(hi).enumerate() {
        let span = u32::from(h) - u32::from(l);
        if span == 0 {
            continue;
        }
        // 严格大于：通道跨度相同时固定取靠前的通道（Y 优先于 T）
        if best.is_none_or(|(_, r)| span > r) {
            best = Some((dim, span));
        }
    }
    best
}

fn weighted_mean(distinct: &[Key], weights: &[u64], bucket: &[usize]) -> Key {
    let total: u64 = bucket.iter().map(|i| weights[*i]).sum();
    let mut sums = [0u64; 4];
    for i in bucket {
        let w = weights[*i];
        for (slot, v) in sums.iter_mut().zip(distinct[*i].map(u64::from)) {
            *slot += v * w;
        }
    }
    let mut out = [0u8; 4];
    for (slot, s) in out.iter_mut().zip(sums) {
        // 半进位；加权均值不会越过通道范围，min(255) 只是不给静默回绕留机会
        *slot = (((s + total / 2) / total).min(255)) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist_of(keys: &[(Key, u64)]) -> HashMap<Key, u64> {
        keys.iter().copied().collect()
    }

    #[test]
    fn exact_policy_keeps_every_color_and_reports_no_loss() {
        let h = hist_of(&[
            ([10, 128, 128, 255], 3u64),
            ([20, 128, 128, 128], 2),
            ([30, 0, 0, 1], 1),
        ]);
        let t = build_table(&h, ColorPolicy::Exact).unwrap();
        assert_eq!(t.entries.len(), 4, "透明槽位 + 3 条");
        assert_eq!(t.stats, None, "没合并就不该报量化");
        let mut seen: Vec<u8> = h.keys().map(|k| t.index_of(*k)).collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3], "每个颜色各占一条");
    }

    #[test]
    fn exact_policy_refuses_the_256th_distinguishable_color() {
        // 255 个 alpha 档 + 透明槽位用满，再多一条可分辨颜色必须失败而不是丢色
        let h: HashMap<Key, u64> = (1..=255u8).map(|a| ([200u8, 100, 50, a], 1u64)).collect();
        build_table(&h, ColorPolicy::Exact).unwrap();
        let mut h2 = h.clone();
        h2.insert([200, 100, 51, 255], 1);
        assert_eq!(h2.len(), 256, "预算刚好差一条");
        let e = build_table(&h2, ColorPolicy::Exact).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0095");
    }

    #[test]
    fn median_cut_stays_inside_budget_and_is_deterministic() {
        let h = wide_histogram(300);
        let a = build_table(&h, ColorPolicy::MedianCut { max_colors: 64 }).unwrap();
        let b = build_table(&h, ColorPolicy::MedianCut { max_colors: 64 }).unwrap();
        assert_eq!(a, b, "同样输入必须得到同样的调色板与索引（golden 前提）");
        assert!(
            a.entries.len() <= 65,
            "预算 64 + 透明槽位：{}",
            a.entries.len()
        );
        let st = a.stats.expect("合并过就必须给出量化统计");
        assert_eq!(st.distinct, 300);
        assert!(st.merged > 0 && st.used > 0);
        assert!(st.worst > 0, "合并了颜色却不报误差，等于把失真藏起来");
        for k in h.keys() {
            let i = a.index_of(*k);
            assert!(i < a.entries.len() as u8, "索引 {i} 超出调色板");
        }
    }

    #[test]
    fn median_cut_is_lossless_when_the_source_already_fits() {
        let h = hist_of(&[([10, 128, 128, 255], 3u64), ([20, 128, 128, 128], 2)]);
        let t = build_table(&h, ColorPolicy::MedianCut { max_colors: 16 }).unwrap();
        assert_eq!(t.stats, None, "没超预算就不该声称做了量化");
        assert_eq!(t.entries.len(), 3);
    }

    #[test]
    fn alpha_ramp_survives_as_a_ramp_not_a_binary_choice() {
        // 255 档不透明的纯白：预算 32 时只能砍 alpha 阶梯，不能把半透明像素变成全透明
        let h: HashMap<Key, u64> = (1..=255u8).map(|a| ([235u8, 128, 128, a], 10u64)).collect();
        let t = build_table(&h, ColorPolicy::MedianCut { max_colors: 32 }).unwrap();
        let alphas: Vec<u8> = t
            .entries
            .iter()
            .skip(1)
            .map(|e| e.alpha)
            .filter(|a| *a != 0)
            .collect();
        assert!(
            alphas.len() >= 8,
            "alpha 阶梯被压成 {} 档，边缘会被二值化：{alphas:?}",
            alphas.len()
        );
        // 最淡的一档不许被合并到"完全透明"：那会让抗锯齿像素直接消失
        let idx = t.index_of([235, 128, 128, 1]);
        assert_ne!(idx, TRANSPARENT_INDEX, "alpha=1 的像素不能被判成全透明");
        assert!(t.entries[idx as usize].alpha >= 1);
    }

    #[test]
    fn transparent_pixels_always_land_on_index_zero() {
        let mut h = hist_of(&[([10, 128, 128, 255], 7u64)]);
        h.insert([0, 0x80, 0x80, 0], 999);
        let t = build_table(&h, ColorPolicy::Exact).unwrap();
        assert_eq!(t.index_of([0, 0x80, 0x80, 0]), TRANSPARENT_INDEX);
        assert_eq!(t.index_of([7, 7, 7, 0]), TRANSPARENT_INDEX, "T==0 就是透明");
        // 透明像素不进预算
        assert_eq!(t.entries.len(), 2);
    }

    #[test]
    fn key_of_normalizes_transparent_and_follows_the_matrix() {
        assert_eq!(
            key_of([255, 0, 0, 0], ColorMatrix::Bt709),
            [0, 0x80, 0x80, 0],
            "T==0 的颜色不管 RGB 是什么都归一到同一个透明槽"
        );
        let white = key_of([255, 255, 255, 255], ColorMatrix::Bt709);
        assert_eq!((white[0], white[3]), (235, 255), "limited range 白 = Y 235");
        // 中性灰在两个矩阵下色差都是 128，所以只能用饱和色来区分矩阵
        let red709 = key_of([255, 0, 0, 255], ColorMatrix::Bt709);
        let red601 = key_of([255, 0, 0, 255], ColorMatrix::Bt601);
        assert_ne!(red709[1], red601[1], "两个矩阵的 Cr 不能混用");
        assert_ne!(red709[2], red601[2], "两个矩阵的 Cb 不能混用");
    }

    #[test]
    fn zero_max_colors_is_a_value_error_not_a_panic() {
        let h = hist_of(&[([10, 128, 128, 255], 7u64)]);
        assert_eq!(
            build_table(&h, ColorPolicy::MedianCut { max_colors: 0 })
                .unwrap_err()
                .code_string(),
            "STK-PGS-0103"
        );
    }

    /// 造一批可分辨颜色：白/黑两条 alpha 阶梯 + 少量色度扰动。
    fn wide_histogram(n: usize) -> HashMap<Key, u64> {
        let mut h: HashMap<Key, u64> = HashMap::new();
        for i in 0..n {
            let a = (i % 250) as u8 + 1;
            let base: [u8; 3] = if i % 2 == 0 {
                [235, 128, 128]
            } else {
                [16, 128, 128]
            };
            let jitter = ((i / 250) * 4) as u8;
            h.insert(
                [base[0], base[1].wrapping_add(jitter), base[2], a],
                1 + i as u64,
            );
        }
        h
    }
}
