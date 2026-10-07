//! 数值字面量的定位与重写（09 §4 B）。
//!
//! ADR-007：原值按精确分数解析，缩放系数是有理数，乘积仍是有理数；只有当结果无法用
//! 有限十进制表达时才舍入，并且舍入必须上报（`Rewritten::approximated`）。
//! 重写只替换数字本身，周围的逗号、空格、括号与写法一律原样保留。

use core_model::time::Rational;

/// 无法精确表达时最多写出的小数位。再多一位对字幕坐标也没有可分辨的意义。
pub const MAX_DECIMALS: u32 = 3;

/// 数值字面量所在的参数种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralMode {
    /// 纯数字参数：`\\pos(320, 480)`、`\\bord2.5`
    Plain,
    /// drawing 文本：`\\clip(\\p1 m 0 0 l 9 9)` 内部、`\\p` 模式下的正文。
    /// libass 把 `\\pN` 当 2 的指数用：`scale_base = 1 << (N-1)`，渲染坐标 =
    /// 数字 / scale_base × screen_scale（`libass/ass_render.c:712-715`、`1188-1191`），
    /// 所以它是精度因子而不是尺寸：Rewrite 缩放坐标数字时 `\\pN` 必须原样保留。
    Drawing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewritten {
    pub text: String,
    /// 结果不能精确表达，发生了舍入
    pub approximated: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RewriteStat {
    /// 实际改写的字面量个数
    pub scaled: usize,
    pub approximated: usize,
    /// 读不成数值（溢出或异常写法）因而原样保留的字面量个数
    pub unparsable: usize,
}

impl RewriteStat {
    pub fn merge(&mut self, other: &RewriteStat) {
        self.scaled += other.scaled;
        self.approximated += other.approximated;
        self.unparsable += other.unparsable;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rewrite {
    pub text: String,
    pub stat: RewriteStat,
}

/// 整数目标（PlayRes / LayoutRes / 样式字段）：缩放后四舍五入取整。
pub fn scale_to_integer(src: &str, factor: &Rational) -> Option<Rewritten> {
    let v = Rational::parse(src).ok()?;
    let s = v.mul(factor)?;
    let r = s.round_i64()?;
    Some(Rewritten {
        text: r.to_string(),
        approximated: Rational::from_int(r) != s,
    })
}

/// 写成尽量短的原文：整数不带小数点；分数在 `MAX_DECIMALS` 位内能精确表达就照写，
/// 否则按 `MAX_DECIMALS` 位四舍五入并标记近似。
pub fn render_ratio(value: &Rational) -> Option<Rewritten> {
    if value.den() == 1 {
        return Some(Rewritten {
            text: value.num().to_string(),
            approximated: false,
        });
    }
    let mut place: i64 = 10;
    for k in 1..=MAX_DECIMALS {
        if let Some(scaled) = value.mul(&Rational::from_int(place)) {
            if scaled.den() == 1 {
                return Some(Rewritten {
                    text: format_fixed(scaled.num(), k),
                    approximated: false,
                });
            }
        }
        place = place.checked_mul(10)?;
    }
    let wide = Rational::from_int(10i64.checked_pow(MAX_DECIMALS)?);
    let scaled = value.mul(&wide)?;
    let r = scaled.round_i64()?;
    Some(Rewritten {
        text: format_fixed(r, MAX_DECIMALS),
        approximated: Rational::from_int(r) != scaled,
    })
}

/// 定位从 `i` 开始的数值字面量，返回其结束字节下标。
/// 允许 `320`、`-3`、`2.5`、`.5`；`1.` 这种只有整数部分的写法不会把点吞进来。
fn literal_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if matches!(b.get(i), Some(b'+') | Some(b'-')) {
        j += 1;
    }
    let int_start = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let int_digits = j - int_start;
    let mut frac_digits = 0usize;
    if b.get(j) == Some(&b'.') {
        let mut k = j + 1;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        frac_digits = k - (j + 1);
        if frac_digits > 0 {
            j = k;
        }
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    Some(j)
}

/// 缩放 `src` 里的数值字面量。`limit` 只改写前 n 个字面量（`\\move` 的后 4 个参数是毫秒）。
pub fn scale_literals(
    src: &str,
    factor: &Rational,
    mode: LiteralMode,
    limit: Option<usize>,
) -> Rewrite {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut stat = RewriteStat::default();
    let mut seen = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if mode == LiteralMode::Drawing && b[i] == b'\\' {
            // drawing 内的 `\pN`（以及任何 `\-alnum` 串）都不是坐标，原样抄走
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_alphanumeric() {
                j += 1;
            }
            out.push_str(src.get(i..j).unwrap_or("\\"));
            i = j;
            continue;
        }
        let Some(end) = literal_end(b, i) else {
            let Some(c) = src[i..].chars().next() else {
                break;
            };
            out.push(c);
            i += c.len_utf8();
            continue;
        };
        if limit.is_some_and(|n| seen >= n) {
            out.push_str(src.get(i..end).unwrap_or(""));
            i = end;
            continue;
        }
        seen += 1;
        let text = src.get(i..end).unwrap_or("");
        let mapped = Rational::parse(text)
            .ok()
            .and_then(|v| v.mul(factor))
            .and_then(|v| render_ratio(&v));
        match mapped {
            Some(r) => {
                if r.text != text {
                    stat.scaled += 1;
                }
                if r.approximated {
                    stat.approximated += 1;
                }
                out.push_str(&r.text);
            }
            None => {
                stat.unparsable += 1;
                out.push_str(text);
            }
        }
        i = end;
    }
    Rewrite { text: out, stat }
}

/// 把已经乘过 10^places 的整数写回定点小数字符串（去掉尾随 0）。
fn format_fixed(v: i64, places: u32) -> String {
    if places == 0 {
        return v.to_string();
    }
    let scale = 10u64.pow(places);
    let neg = v < 0;
    let a = v.unsigned_abs();
    let ip = a / scale;
    let fp = a % scale;
    let mut s = String::new();
    if neg && (ip != 0 || fp != 0) {
        s.push('-');
    }
    s.push_str(&ip.to_string());
    if fp != 0 {
        let mut f = fp.to_string();
        while (f.len() as u32) < places {
            f.insert(0, '0');
        }
        while f.ends_with('0') {
            f.pop();
        }
        s.push('.');
        s.push_str(&f);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn integer_target_rounds_half_up_and_flags() {
        let two = Rational::from_int(2);
        let exact = scale_to_integer("1920", &two).unwrap();
        assert_eq!(exact.text, "3840");
        assert!(!exact.approximated);
        let off = scale_to_integer("1000", &r(1920, 1000)).unwrap();
        assert_eq!(off.text, "1920");
        assert!(!off.approximated);
        let rounded = scale_to_integer("25", &r(3, 2)).unwrap();
        assert_eq!(rounded.text, "38");
        assert!(rounded.approximated);
        // 负号：half-up 远离 0
        assert_eq!(scale_to_integer("-25", &r(3, 2)).unwrap().text, "-38");
        assert_eq!(scale_to_integer("x", &two), None);
    }

    #[test]
    fn decimal_rendering_is_shortest_exact() {
        assert_eq!(render_ratio(&r(3840, 1)).unwrap().text, "3840");
        assert_eq!(render_ratio(&r(75, 2)).unwrap().text, "37.5");
        assert_eq!(render_ratio(&r(1, 8)).unwrap().text, "0.125");
        assert_eq!(render_ratio(&r(-1, 4)).unwrap().text, "-0.25");
        let inexact = render_ratio(&r(1, 3)).unwrap();
        assert_eq!(inexact.text, "0.333");
        assert!(inexact.approximated);
        // 1/16 = 0.0625 超出 3 位精度 -> 舍入
        assert_eq!(render_ratio(&r(1, 16)).unwrap().text, "0.063");
    }

    #[test]
    fn plain_mode_keeps_every_other_byte() {
        let two = Rational::from_int(2);
        let got = scale_literals("(320, 480)", &two, LiteralMode::Plain, None);
        assert_eq!(got.text, "(640, 960)");
        assert_eq!(got.stat.scaled, 2);
        assert_eq!(got.stat.unparsable, 0);
        let odd = scale_literals("( 1.5 ,2. )", &two, LiteralMode::Plain, None);
        assert_eq!(odd.text, "( 3 ,4. )");
        let sign = scale_literals("(-10,.5)", &two, LiteralMode::Plain, None);
        assert_eq!(sign.text, "(-20,1)");
    }

    #[test]
    fn limit_protects_move_time_arguments() {
        let two = Rational::from_int(2);
        // \move(x1,y1,x2,y2,t1,t2,t3)：后 4 个是毫秒，绝不能按分辨率缩放
        let got = scale_literals(
            "(10,20,30,40,100,900,500)",
            &two,
            LiteralMode::Plain,
            Some(4),
        );
        assert_eq!(got.text, "(20,40,60,80,100,900,500)");
        assert_eq!(got.stat.scaled, 4);
    }

    #[test]
    fn drawing_mode_preserves_precision_factor() {
        let two = Rational::from_int(2);
        let got = scale_literals("\\p1 m 0 0 l 9 9\\p0", &two, LiteralMode::Drawing, None);
        assert_eq!(got.text, "\\p1 m 0 0 l 18 18\\p0");
        // 0 缩放后仍是 0：只有文本真的变化才计入 scaled
        assert_eq!(got.stat.scaled, 2);
        let scaled_up = scale_literals("\\p8 m 80 80 l 160 160", &two, LiteralMode::Drawing, None);
        assert_eq!(scaled_up.text, "\\p8 m 160 160 l 320 320");
    }

    #[test]
    fn unparsable_literals_are_kept_and_counted() {
        let big = "99999999999999999999999";
        let got = scale_literals(
            &format!("({big},10)"),
            &Rational::from_int(2),
            LiteralMode::Plain,
            None,
        );
        assert_eq!(got.text, format!("({big},20)"));
        assert_eq!(got.stat.unparsable, 1);
        assert_eq!(got.stat.scaled, 1);
    }

    #[test]
    fn identity_factor_is_byte_for_byte() {
        let one = Rational::ONE;
        for src in [
            "(320, 480)",
            "\\p1 m 0 0 l 9 9\\p0",
            "(-1.5,2.5)",
            "no digits here",
        ] {
            let m = if src.contains('m') {
                LiteralMode::Drawing
            } else {
                LiteralMode::Plain
            };
            assert_eq!(scale_literals(src, &one, m, None).text, src);
        }
    }

    #[test]
    fn format_fixed_handles_padding_and_zero() {
        assert_eq!(format_fixed(375, 1), "37.5");
        assert_eq!(format_fixed(5, 3), "0.005");
        assert_eq!(format_fixed(-5, 3), "-0.005");
        assert_eq!(format_fixed(0, 3), "0");
        assert_eq!(format_fixed(1000, 3), "1");
        assert_eq!(format_fixed(i64::MIN, 0), i64::MIN.to_string());
    }
}
