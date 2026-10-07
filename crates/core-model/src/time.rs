//! 时间模型：有理数秒 + 精确 FPS + 各格式时间表示的唯一换算出口。
//! ADR-007：核心不参与 float 时间运算；float 仅在显示边界出现。

use core::fmt;

use crate::error::{Domain, StkError};

/// MPEG-PGS 时间戳上限：33 bit。
pub const PTS_MAX_TICKS: i64 = 1 << 33;

fn overflow() -> StkError {
    StkError::simple(Domain::Transform, 3, "时间数值溢出")
}

/// 有限有理数。不变量：`den > 0` 且 `gcd(|num|, den) == 1`；零表示为 `0/1`。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd128(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// 编译期构造，仅用于已确认互质的常量。
    pub const fn from_const(num: i64, den: i64) -> Rational {
        Rational { num, den }
    }

    pub fn new(num: i64, den: i64) -> Result<Rational, StkError> {
        if den == 0 {
            return Err(StkError::simple(Domain::Transform, 2, "分母为 0"));
        }
        // 用 i128 归一：i64::MIN 取反会在 i64 内溢出
        let (mut n, mut d): (i128, i128) = if den < 0 {
            (-(num as i128), -(den as i128))
        } else {
            (num as i128, den as i128)
        };
        let g = gcd128(if n < 0 { -n } else { n }, if d < 0 { -d } else { d }).max(1);
        n /= g;
        d /= g;
        let n =
            i64::try_from(n).map_err(|_| StkError::simple(Domain::Transform, 3, "时间数值溢出"))?;
        let d =
            i64::try_from(d).map_err(|_| StkError::simple(Domain::Transform, 3, "时间数值溢出"))?;
        Ok(Rational { num: n, den: d })
    }

    pub const fn from_int(v: i64) -> Rational {
        Rational { num: v, den: 1 }
    }
    pub fn num(&self) -> i64 {
        self.num
    }
    pub fn den(&self) -> i64 {
        self.den
    }
    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    fn from_i128(n: i128, d: i128) -> Option<Rational> {
        if d == 0 {
            return None;
        }
        let (mut x, mut y) = (if n < 0 { -n } else { n }, if d < 0 { -d } else { d });
        while y != 0 {
            let t = x % y;
            x = y;
            y = t;
        }
        let g = x.max(1);
        let (n, d) = (n / g, d / g);
        let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
        Some(Rational {
            num: i64::try_from(n).ok()?,
            den: i64::try_from(d).ok()?,
        })
    }

    pub fn add(&self, o: &Rational) -> Option<Rational> {
        Self::from_i128(
            i128::from(self.num) * i128::from(o.den) + i128::from(o.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(o.den),
        )
    }
    pub fn sub(&self, o: &Rational) -> Option<Rational> {
        Self::from_i128(
            i128::from(self.num) * i128::from(o.den) - i128::from(o.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(o.den),
        )
    }
    pub fn mul(&self, o: &Rational) -> Option<Rational> {
        Self::from_i128(
            i128::from(self.num) * i128::from(o.num),
            i128::from(self.den) * i128::from(o.den),
        )
    }
    pub fn div(&self, o: &Rational) -> Option<Rational> {
        if o.num == 0 {
            return None;
        }
        Self::from_i128(
            i128::from(self.num) * i128::from(o.den),
            i128::from(self.den) * i128::from(o.num),
        )
    }
    pub fn neg(&self) -> Option<Rational> {
        Rational::new(-self.num, self.den).ok()
    }

    /// 仅供显示/日志。
    pub fn to_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// half-up 舍入（负数按绝对值远离零）。
    pub fn round_i64(&self) -> Option<i64> {
        let n = i128::from(self.num);
        let d = i128::from(self.den);
        let v = if n >= 0 {
            (2 * n + d) / (2 * d)
        } else {
            -((2 * (-n) + d) / (2 * d))
        };
        i64::try_from(v).ok()
    }
    pub fn floor_i64(&self) -> i64 {
        if self.num >= 0 {
            self.num / self.den
        } else {
            -(((-self.num) + self.den - 1) / self.den)
        }
    }

    /// 解析 `24`、`24000/1001`、`23.976`（十进制按精确分数保留，不经过 float）。
    pub fn parse(s: &str) -> Result<Rational, StkError> {
        let s = s.trim();
        if let Some((a, b)) = s.split_once('/') {
            let a = a
                .trim()
                .parse::<i64>()
                .map_err(|_| StkError::simple(Domain::Input, 11, "无法解析比率"))?;
            let b = b
                .trim()
                .parse::<i64>()
                .map_err(|_| StkError::simple(Domain::Input, 11, "无法解析比率"))?;
            return Rational::new(a, b);
        }
        if s.contains('.') {
            let (sign, body) = match s.as_bytes().first() {
                Some(b'-') => (-1i64, &s[1..]),
                Some(b'+') => (1, &s[1..]),
                _ => (1, s),
            };
            let dot = body
                .find('.')
                .ok_or_else(|| StkError::simple(Domain::Input, 11, "小数格式非法"))?;
            let ip = &body[..dot];
            let fp = &body[dot + 1..];
            if ip.is_empty() && fp.is_empty() {
                return Err(StkError::simple(Domain::Input, 11, "小数格式非法"));
            }
            if fp.chars().any(|c| !c.is_ascii_digit()) || ip.chars().any(|c| !c.is_ascii_digit()) {
                return Err(StkError::simple(Domain::Input, 11, "小数格式非法"));
            }
            let ip_v: i64 = if ip.is_empty() {
                0
            } else {
                ip.parse()
                    .map_err(|_| StkError::simple(Domain::Input, 11, "小数整数部分非法"))?
            };
            let fp_v: i64 = if fp.is_empty() {
                0
            } else {
                fp.parse()
                    .map_err(|_| StkError::simple(Domain::Input, 11, "小数小数部分非法"))?
            };
            let mut scale: i64 = 1;
            for _ in 0..fp.len() {
                scale = scale.checked_mul(10).ok_or_else(overflow)?;
            }
            let num = (ip_v.checked_mul(scale).ok_or_else(overflow)? + fp_v)
                .checked_mul(sign)
                .ok_or_else(overflow)?;
            return Rational::new(num, scale).map_err(|_| overflow());
        }
        let v = s
            .parse::<i64>()
            .map_err(|_| StkError::simple(Domain::Input, 11, "无法解析整数"))?;
        Ok(Rational::from_int(v))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, o: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl core::cmp::Ord for Rational {
    fn cmp(&self, o: &Self) -> core::cmp::Ordering {
        let lhs = i128::from(self.num) * i128::from(o.den);
        let rhs = i128::from(o.num) * i128::from(self.den);
        lhs.cmp(&rhs)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}
impl fmt::Debug for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// 时间（有理数秒）。允许为负，仅用于预览 offset；落盘前由调用方归一。
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Time(Rational);

impl Time {
    pub const ZERO: Time = Time(Rational::ZERO);

    pub fn from_rational(r: Rational) -> Time {
        Time(r)
    }
    pub const fn from_secs(v: i64) -> Time {
        Time(Rational::from_int(v))
    }
    pub fn from_millis(v: i64) -> Result<Time, StkError> {
        Ok(Time(rat(v, 1000)?))
    }

    /// ASS：`H:MM:SS.cc`。
    pub fn from_hcs(h: i64, m: i64, s: i64, cs: i64) -> Result<Time, StkError> {
        if h < 0 || !(0..60).contains(&m) || !(0..60).contains(&s) || !(0..100).contains(&cs) {
            return Err(StkError::simple(Domain::Parse, 30, "ASS 时间字段越界"));
        }
        Ok(Time(
            rat(h * 3600 + m * 60 + s, 1)?
                .add(&rat(cs, 100)?)
                .ok_or_else(overflow)?,
        ))
    }

    /// SRT：`HH:MM:SS,mmm`。
    pub fn from_hms_millis(h: i64, m: i64, s: i64, ms: i64) -> Result<Time, StkError> {
        if h < 0 || !(0..60).contains(&m) || !(0..60).contains(&s) || !(0..1000).contains(&ms) {
            return Err(StkError::simple(Domain::Parse, 31, "SRT 时间字段越界"));
        }
        Ok(Time(
            rat(h * 3600 + m * 60 + s, 1)?
                .add(&rat(ms, 1000)?)
                .ok_or_else(overflow)?,
        ))
    }

    pub fn as_rational(&self) -> Rational {
        self.0
    }
    pub fn is_negative(&self) -> bool {
        self.0.num() < 0
    }
    pub fn add(&self, o: &Time) -> Result<Time, StkError> {
        Ok(Time(self.0.add(&o.0).ok_or_else(overflow)?))
    }
    pub fn sub(&self, o: &Time) -> Result<Time, StkError> {
        Ok(Time(self.0.sub(&o.0).ok_or_else(overflow)?))
    }
    pub fn scale(&self, r: &Rational) -> Result<Time, StkError> {
        Ok(Time(self.0.mul(r).ok_or_else(overflow)?))
    }

    /// PGS 90 kHz ticks；负值/超 33bit 直接报错，不回绕。
    pub fn to_pts_ticks(&self) -> Result<i64, StkError> {
        let v = self
            .0
            .mul(&Rational::from_int(90_000))
            .ok_or_else(overflow)?
            .round_i64()
            .ok_or_else(overflow)?;
        if v < 0 {
            return Err(StkError::simple(Domain::Pgs, 1, "PTS 为负"));
        }
        if v >= PTS_MAX_TICKS {
            return Err(StkError::simple(Domain::Pgs, 2, "PTS 超过 33bit 上限"));
        }
        Ok(v)
    }
    pub fn from_pts_ticks(ticks: i64) -> Result<Time, StkError> {
        if ticks < 0 {
            return Err(StkError::simple(Domain::Pgs, 1, "PTS 为负"));
        }
        Ok(Time(rat(ticks, 90_000)?))
    }

    /// Frame Snap 基元：round(t*fps)/fps。
    pub fn snap_to_frame(&self, fps: Fps) -> Result<Time, StkError> {
        let frames = self
            .0
            .mul(&fps.0)
            .ok_or_else(overflow)?
            .round_i64()
            .ok_or_else(overflow)?;
        Ok(Time(
            Rational::from_int(frames)
                .div(&fps.0)
                .ok_or_else(overflow)?,
        ))
    }

    pub fn to_ass_string(&self) -> Result<String, StkError> {
        let cs = total_part(self, 100)?;
        Ok(format!(
            "{}:{:02}:{:02}.{:02}",
            cs / 360_000,
            (cs % 360_000) / 6_000,
            (cs % 6_000) / 100,
            cs % 100
        ))
    }

    pub fn to_srt_string(&self) -> Result<String, StkError> {
        let ms = total_part(self, 1000)?;
        Ok(format!(
            "{:02}:{:02}:{:02},{:03}",
            ms / 3_600_000,
            (ms % 3_600_000) / 60_000,
            (ms % 60_000) / 1000,
            ms % 1000
        ))
    }

    /// BDN timecode：SS 来自真实整秒，FF 来自同一时间值的小数部分（单一舍入源，长片无漂移）。
    pub fn to_bdn_timecode(&self, fps: Fps) -> Result<BdnTimecode, StkError> {
        if self.is_negative() {
            return Err(StkError::simple(Domain::Bdn, 3, "BDN 时间码不能为负"));
        }
        let nominal = fps.nominal()?;
        // 量化只在这里发生一次：N = floor(t × fps_exact)。
        // 参照实现 ass2bdnxml `mk_timecode()`（SMPTE non-drop）就是从帧号取模得到 FF/SS/MM/HH，
        // 秒字段来自帧号而不是墙上秒；用"墙上秒 + 另一套舍入的 FF"就是 10 §7 禁止的双舍入源。
        let index = self
            .0
            .mul(&fps.as_rational())
            .ok_or_else(overflow)?
            .floor_i64();
        BdnTimecode::from_frame_index(index, nominal)
    }
}

fn total_part(t: &Time, scale: i64) -> Result<i64, StkError> {
    let v =
        t.0.mul(&Rational::from_int(scale))
            .ok_or_else(overflow)?
            .floor_i64();
    if v < 0 {
        return Err(StkError::simple(Domain::Output, 3, "时间不能为负"));
    }
    Ok(v)
}

fn rat(n: i64, d: i64) -> Result<Rational, StkError> {
    Rational::new(n, d).map_err(|_| StkError::simple(Domain::Transform, 3, "时间数值溢出"))
}

/// 帧率：预设保存精确 rational。
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Fps(Rational);

impl Fps {
    pub const NTSC_23976: Fps = Fps(Rational::from_const(24000, 1001));
    pub const FILM_24: Fps = Fps(Rational::from_int(24));
    pub const PAL_25: Fps = Fps(Rational::from_int(25));
    pub const NTSC_2997: Fps = Fps(Rational::from_const(30000, 1001));
    pub const NTSC_30: Fps = Fps(Rational::from_int(30));
    pub const PAL_50: Fps = Fps(Rational::from_int(50));
    pub const NTSC_5994: Fps = Fps(Rational::from_const(60000, 1001));
    pub const NTSC_60: Fps = Fps(Rational::from_int(60));

    pub const PRESETS: &'static [(&'static str, Fps)] = &[
        ("23.976", Fps::NTSC_23976),
        ("24", Fps::FILM_24),
        ("25", Fps::PAL_25),
        ("29.97", Fps::NTSC_2997),
        ("30", Fps::NTSC_30),
        ("50", Fps::PAL_50),
        ("59.94", Fps::NTSC_5994),
        ("60", Fps::NTSC_60),
    ];

    pub fn new(r: Rational) -> Result<Fps, StkError> {
        if r <= Rational::ZERO {
            return Err(StkError::simple(Domain::Input, 12, "FPS 必须为正"));
        }
        if r >= Rational::from_int(1000) {
            return Err(StkError::simple(Domain::Input, 12, "FPS 超出合理范围"));
        }
        Ok(Fps(r))
    }

    /// 预设名优先（"23.976" → 24000/1001），其次 rational / 十进制。
    pub fn from_label(s: &str) -> Result<Fps, StkError> {
        let t = s.trim();
        for (label, f) in Self::PRESETS {
            if *label == t {
                return Ok(*f);
            }
        }
        Fps::new(Rational::parse(t)?)
    }

    pub fn as_rational(&self) -> Rational {
        self.0
    }

    /// BDN FF 字段的整数基：23.976 → 24。
    pub fn nominal(&self) -> Result<i64, StkError> {
        let n = self.0.round_i64().ok_or_else(overflow)?;
        if n <= 0 {
            return Err(StkError::simple(Domain::Input, 12, "FPS 取整后为 0"));
        }
        Ok(n)
    }

    pub fn to_label(&self) -> String {
        for (label, f) in Self::PRESETS {
            if *f == *self {
                return (*label).to_string();
            }
        }
        format!("{}", self.0)
    }

    /// Speed Conversion 倍率：t_new = t_old * source / target。
    pub fn speed_ratio(&self, target: Fps) -> Result<Rational, StkError> {
        self.0.div(&target.0).ok_or_else(overflow)
    }
}

impl Default for Fps {
    fn default() -> Self {
        Fps::NTSC_23976
    }
}
impl fmt::Display for Fps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_label())
    }
}
impl fmt::Debug for Fps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BdnTimecode {
    pub h: i64,
    pub m: i64,
    pub s: i64,
    pub ff: i64,
}

impl BdnTimecode {
    /// 从帧号构造 SMPTE 非丢帧时间码：`ff = N % nominal`，秒/分/时依次除下去
    /// （与 ass2bdnxml `mk_timecode()` 一致；超过 99 小时参考实现直接拒绝，我们也拒绝）。
    pub fn from_frame_index(index: i64, nominal: i64) -> Result<BdnTimecode, StkError> {
        if index < 0 {
            return Err(StkError::simple(Domain::Bdn, 3, "BDN 时间码不能为负"));
        }
        if nominal <= 0 {
            return Err(StkError::simple(Domain::Bdn, 5, "帧率基准非法"));
        }
        let mut rest = index;
        let ff = rest % nominal;
        rest /= nominal;
        let s = rest % 60;
        rest /= 60;
        let m = rest % 60;
        rest /= 60;
        let h = rest;
        if h > 99 {
            return Err(
                StkError::simple(Domain::Bdn, 7, "BDN 时间码超过 99:59:59:99")
                    .with_detail(index.to_string()),
            );
        }
        Ok(BdnTimecode { h, m, s, ff })
    }

    /// 时间码 → 帧号（`to_time` 的同一套算术，因此往返必然一致）。
    pub fn to_frame_index(&self, nominal: i64) -> Result<i64, StkError> {
        self.validate_fields(nominal)?;
        Ok((((self.h * 60) + self.m) * 60 + self.s)
            .checked_mul(nominal)
            .ok_or_else(overflow)?
            + self.ff)
    }

    fn validate_fields(&self, nominal: i64) -> Result<(), StkError> {
        if self.h < 0 || !(0..60).contains(&self.m) || !(0..60).contains(&self.s) {
            return Err(StkError::simple(Domain::Bdn, 4, "BDN 时间码分/秒越界"));
        }
        if nominal <= 0 || self.ff < 0 || self.ff >= nominal {
            return Err(StkError::simple(
                Domain::Bdn,
                5,
                "BDN 帧字段必须在 [0, nominal) 内",
            ));
        }
        Ok(())
    }

    /// 回读：`t = N / fps_exact`。在帧网格上是精确的，没有第二种舍入源（10 §7）。
    pub fn to_time(&self, fps: Fps) -> Result<Time, StkError> {
        let nominal = fps.nominal()?;
        let index = self.to_frame_index(nominal)?;
        let t = Rational::from_int(index)
            .div(&fps.as_rational())
            .ok_or_else(overflow)?;
        Ok(Time(t))
    }

    pub fn parse(s: &str) -> Result<BdnTimecode, StkError> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 4 {
            return Err(StkError::simple(
                Domain::Bdn,
                6,
                "BDN 时间码必须是 HH:MM:SS:FF",
            ));
        }
        let mut v = [0i64; 4];
        for (i, p) in parts.iter().enumerate() {
            v[i] = p
                .trim()
                .parse::<i64>()
                .map_err(|_| StkError::simple(Domain::Bdn, 6, "BDN 时间码数字非法"))?;
            if v[i] < 0 {
                return Err(StkError::simple(Domain::Bdn, 6, "BDN 时间码为负"));
            }
        }
        Ok(BdnTimecode {
            h: v[0],
            m: v[1],
            s: v[2],
            ff: v[3],
        })
    }
}

impl fmt::Display for BdnTimecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02}:{:02}:{:02}:{:02}",
            self.h, self.m, self.s, self.ff
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_normalizes_and_orders() {
        assert_eq!(
            Rational::new(48000, 2002).unwrap(),
            Rational::new(24000, 1001).unwrap()
        );
        assert!(Rational::new(24000, 1001).unwrap() < Rational::from_int(24));
        assert_eq!(Rational::new(1, -2).unwrap(), Rational::new(-1, 2).unwrap());
        assert!(Rational::new(1, 0).is_err());
    }

    #[test]
    fn decimal_parse_is_exact() {
        assert_eq!(
            Rational::parse("23.976").unwrap(),
            Rational::new(2997, 125).unwrap()
        );
        assert_eq!(
            Rational::parse("0.5").unwrap(),
            Rational::new(1, 2).unwrap()
        );
        assert_eq!(
            Rational::parse("-1.25").unwrap(),
            Rational::new(-5, 4).unwrap()
        );
        assert_eq!(Rational::parse("24/1").unwrap(), Rational::from_int(24));
        assert!(Rational::parse("1.2.3").is_err());
        assert!(Rational::parse("abc").is_err());
    }

    #[test]
    fn rounding_and_floor_are_half_up_and_toward_minus_inf() {
        assert_eq!(Rational::new(5, 2).unwrap().round_i64(), Some(3));
        assert_eq!(Rational::new(-5, 2).unwrap().round_i64(), Some(-3));
        assert_eq!(Rational::new(1, 3).unwrap().round_i64(), Some(0));
        assert_eq!(Rational::new(-1, 3).unwrap().floor_i64(), -1);
        assert_eq!(Rational::new(7, 2).unwrap().floor_i64(), 3);
    }

    #[test]
    fn checked_overflow_never_wraps() {
        let big = Rational::from_int(i64::MAX);
        assert!(big.mul(&big).is_none());
        assert!(Rational::from_int(1).div(&Rational::ZERO).is_none());
    }

    /// 回归：i64::MIN 取反在 i64 内会溢出（codec-srt 审查时发现）
    #[test]
    fn int_min_extremes_do_not_overflow() {
        assert_eq!(Rational::new(i64::MIN, 1).unwrap().num(), i64::MIN);
        assert_eq!(
            Rational::new(i64::MIN, 2).unwrap(),
            Rational::from_int(i64::MIN / 2)
        );
        assert_eq!(
            Rational::new(i64::MIN, -2).unwrap(),
            Rational::from_int(i64::MIN / -2)
        );
        // 无法用 i64 表示的分子必须报错，而不是回绕
        assert!(Rational::new(i64::MIN, -1).is_err());
        assert!(Rational::new(i64::MIN, i64::MIN).unwrap().num() > 0);
    }

    #[test]
    fn ass_srt_print_roundtrip() {
        let t = Time::from_hcs(0, 0, 3, 50).unwrap();
        assert_eq!(t.to_ass_string().unwrap(), "0:00:03.50");
        assert_eq!(t.to_srt_string().unwrap(), "00:00:03,500");
        let u = Time::from_hms_millis(1, 2, 3, 456).unwrap();
        assert_eq!(u.to_srt_string().unwrap(), "01:02:03,456");
        assert!(Time::from_hcs(0, 0, 0, 100).is_err());
        assert!(Time::from_millis(-1).unwrap().to_srt_string().is_err());
    }

    #[test]
    fn pts_exact_90khz() {
        assert_eq!(
            Time::from_millis(1000).unwrap().to_pts_ticks().unwrap(),
            90_000
        );
        assert_eq!(
            Time::from_millis(500).unwrap().to_pts_ticks().unwrap(),
            45_000
        );
        assert_eq!(
            Time::from_pts_ticks(90_000).unwrap(),
            Time::from_millis(1000).unwrap()
        );
        assert!(Time::from_millis(-1).unwrap().to_pts_ticks().is_err());
        // PTS 为 33 bit：约 95443 秒（26.5 小时）之后才溢出
        assert!(Time::from_secs(90_000).to_pts_ticks().is_ok());
        assert!(Time::from_secs(100_000).to_pts_ticks().is_err());
    }

    /// BDN 时间码是"帧号"，不是墙上时钟。23.976 下 1.000 秒是第 23 帧（N = trunc(23.976)），
    /// 而 "00:00:01:00" 对应第 24 帧 = 1.001 秒。参照 ass2bdnxml `mk_timecode()`。
    #[test]
    fn bdn_timecode_is_a_frame_index_not_wall_clock() {
        let fps = Fps::NTSC_23976;
        assert_eq!(
            Time::from_secs(1).to_bdn_timecode(fps).unwrap().to_string(),
            "00:00:00:23"
        );
        assert_eq!(
            Time::from_millis(1001)
                .unwrap()
                .to_bdn_timecode(fps)
                .unwrap()
                .to_string(),
            "00:00:01:00",
            "24 帧恰好是 1.001 秒"
        );
        assert_eq!(
            Time::from_millis(3500)
                .unwrap()
                .to_bdn_timecode(fps)
                .unwrap()
                .to_string(),
            "00:00:03:11",
            "trunc(3.5 × 23.976) = 83 = 3*24 + 11"
        );
        assert_eq!(
            Time::from_millis(1990)
                .unwrap()
                .to_bdn_timecode(fps)
                .unwrap()
                .to_string(),
            "00:00:01:23"
        );
    }

    #[test]
    fn bdn_timecode_roundtrips_exactly_on_the_frame_grid() {
        let fps = Fps::NTSC_23976;
        let nominal = fps.nominal().unwrap();
        // 两小时长的帧网格（172800 帧）逐帧往返必须精确相等：这就是"单一舍入源"的可执行定义。
        for n in (0..172_800i64).step_by(7) {
            let tc = BdnTimecode::from_frame_index(n, nominal).unwrap();
            let t = tc.to_time(fps).unwrap();
            assert_eq!(
                tc.to_string(),
                t.to_bdn_timecode(fps).unwrap().to_string(),
                "第 {n} 帧往返不一致"
            );
            assert_eq!(tc.to_frame_index(nominal).unwrap(), n);
        }
        // 墙上 2 小时在 23.976 帧号时间码上不是 02:00:00:00（后者是第 172800 帧 = 7207.2 秒）
        assert_ne!(
            Time::from_secs(7200)
                .to_bdn_timecode(fps)
                .unwrap()
                .to_string(),
            "02:00:00:00"
        );
        assert_eq!(
            BdnTimecode::from_frame_index(172_800, nominal)
                .unwrap()
                .to_string(),
            "02:00:00:00"
        );
        assert!(BdnTimecode::parse("00:00:03:24")
            .unwrap()
            .to_time(fps)
            .is_err());
        assert!(BdnTimecode::parse("0:00:03").is_err());
        assert!(BdnTimecode::from_frame_index(100 * 3600 * 24, nominal).is_err());
    }

    #[test]
    fn fps_presets_are_exact_rationals() {
        assert_eq!(
            Fps::from_label("23.976").unwrap().as_rational(),
            Rational::new(24000, 1001).unwrap()
        );
        assert_eq!(Fps::NTSC_23976.to_label(), "23.976");
        assert_eq!(Fps::NTSC_23976.nominal().unwrap(), 24);
        assert_eq!(Fps::PAL_50.nominal().unwrap(), 50);
        assert_eq!(Fps::from_label("50/2").unwrap(), Fps::PAL_25);
        assert_eq!(Fps::from_label("0").unwrap_or(Fps::FILM_24), Fps::FILM_24);
        assert!(Fps::new(Rational::from_int(0)).is_err());
    }

    #[test]
    fn speed_ratio_and_frame_snap() {
        let r = Fps::NTSC_23976.speed_ratio(Fps::PAL_25).unwrap();
        assert_eq!(r, Rational::new(24000, 25025).unwrap());
        let snapped = Time::from_millis(12345)
            .unwrap()
            .snap_to_frame(Fps::NTSC_2997)
            .unwrap();
        assert_eq!(snapped.snap_to_frame(Fps::NTSC_2997).unwrap(), snapped);
        // snap 不改变总时长语义：同一帧网格内
        assert!(
            (snapped
                .as_rational()
                .sub(&Time::from_millis(12345).unwrap().as_rational())
                .unwrap()
                .abs())
                < Rational::new(1, 24).unwrap()
        );
    }

    #[test]
    fn long_run_has_no_accumulated_drift() {
        // AT-TIME-001 的最小对应：23.976 → 25 缩放后，长片末尾时间仍可精确回推
        let src = Fps::NTSC_23976;
        let dst = Fps::PAL_25;
        let end = Time::from_secs(2 * 3600);
        let scaled = end.scale(&src.speed_ratio(dst).unwrap()).unwrap();
        let back = scaled.scale(&dst.speed_ratio(src).unwrap()).unwrap();
        assert_eq!(back, end);
    }

    impl Rational {
        fn abs(&self) -> Rational {
            if self.num < 0 {
                Rational {
                    num: -self.num,
                    den: self.den,
                }
            } else {
                *self
            }
        }
    }
}
