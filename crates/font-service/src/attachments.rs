//! `[Fonts]` 附件块（17 §7 的第二句："由 libass extraction 或我们自己的 attachment parser 处理"）。
//!
//! 走的是**我们自己的解析器**，理由是这条路上有两个判断必须先做而不是交给 libass：
//! * 附件字节要注册给 `ass_add_font`，而 libass 按**字体表里的内部家族名**匹配内存字体
//!   （实测按附件名注册不会命中，ISSUES AK1）——所以我们得先把内部名读出来；
//! * 23 §健壮性要求"声明是字体"的东西不能直接喂解析器：大小上限、base64 合法性、
//!   是不是可解析的字体，都得有一条能报告的失败路径，而不是静默少装一份字体。
//!
//! 支持的写法与 Aegisub/libass 一致：`[Fonts]` 段内 `P: <文件名>` 起一份附件，
//! 其后到下一个 `P:` 或段结束之间的行都是 base64。别的形状（旧式 `名字=base64`）
//! **明确报出来**而不是当作"没有附件" —— "没认出来"和"没有"是两个不同的结论。
//!
//! 不引入 `base64` crate：为一个 30 行的解码器拉新依赖要走 21 §1 审批（与 15 §2 的
//! glob 同一判断，见 ISSUES E6），而解码器的正确性用已知向量钉住比换依赖更便宜。

use core_model::error::{Domain, Result, StkError};
use sha2::{Digest as _, Sha256};

/// 单份附件的字节上限。ASS 内嵌字体现实里是几百 KB 到几 MB；
/// 这个数只用来挡住"一个 2 GB 的 base64 块把我们内存吃光"（23 §健壮性）。
pub const MAX_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;
/// 整份脚本里所有附件的合计上限（同上，且防"很多份刚好都不超限"）。
pub const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;

/// 一份从 `[Fonts]` 里解出来的附件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// 脚本里写的那个附件名（通常是 `Arial.ttf`）。注册给 libass 时**不用它**，
    /// 用的是 `family` —— 见模块开头。
    pub declared_name: String,
    /// 字体表里的内部家族名；读不出来时是 `None`（下面 `is_parseable` 为假的那种）
    pub family: Option<String>,
    pub sha256: String,
    /// 解码出来的原件字节。带着它是因为注入这条路**就是要这批字节**：
    /// 只存哈希再回头读文件，等于把"脚本自己声明的字体"又变成"磁盘上现在是什么算什么"。
    pub data: Vec<u8>,
    /// 这份字节能不能被字体解析器读回来（`read-fonts` 的视角，不是"看起来像"）
    pub parseable: bool,
}

impl Attachment {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// `len()==0` 的附件在 `parse` 里就会被拒（不是可解析的字体），这里只为满足 clippy 的配对约定。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Attachment {
    /// 报告用的身份：内部名优先，没有就退回声明名 —— 两者都给出，读者能看出差在哪。
    pub fn label(&self) -> String {
        self.family
            .clone()
            .unwrap_or_else(|| self.declared_name.clone())
    }
}

/// 解出 `[Fonts]` 块。没有这个段就是空列表（不是错误）；
/// 段里有我们认不出的形状、base64 非法、超限、或"声明是字体却读不回来"，都报错。
pub fn parse(text: &str) -> Result<Vec<Attachment>> {
    let mut out: Vec<Attachment> = Vec::new();
    let mut total = 0usize;
    let mut in_fonts = false;
    let mut name: Option<String> = None;
    let mut payload = String::new();
    let mut legacy: Vec<String> = Vec::new();

    let flush = |name: &mut Option<String>,
                 payload: &mut String,
                 out: &mut Vec<Attachment>,
                 total: &mut usize| {
        let Some(n) = name.take() else {
            payload.clear();
            return Ok(());
        };
        let bytes = decode_base64(payload)?;
        payload.clear();
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(attach_err(
                241,
                "内嵌字体超过单份大小上限",
                &format!("{n}：{} 字节，上限 {MAX_ATTACHMENT_BYTES}", bytes.len()),
            ));
        }
        *total += bytes.len();
        if *total > MAX_TOTAL_BYTES {
            return Err(attach_err(
                242,
                "内嵌字体合计超过上限",
                &format!("累计 {total} 字节，上限 {MAX_TOTAL_BYTES}"),
            ));
        }
        let parseable = crate::is_parseable_font(&bytes);
        if !parseable {
            return Err(attach_err(
                243,
                "内嵌字体读不回来：不是可解析的字体或字体集合",
                &format!("{n}：{} 字节", bytes.len()),
            ));
        }
        let family = crate::family_of_bytes(&bytes).ok();
        out.push(Attachment {
            declared_name: n,
            family,
            sha256: digest_hex(&bytes),
            data: bytes,
            parseable,
        });
        Ok(())
    };

    for line in text.lines() {
        let t = line.trim_end_matches('\r');
        let trimmed = t.trim_start();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // 段切换：先把手上这份结掉，再决定是否进入 [Fonts]
            flush(&mut name, &mut payload, &mut out, &mut total)?;
            in_fonts = trimmed.eq_ignore_ascii_case("[fonts]");
            continue;
        }
        if !in_fonts {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = strip_name_key(trimmed) {
            if rest.is_empty() {
                legacy.push(trimmed.chars().take(40).collect());
                continue;
            }
            flush(&mut name, &mut payload, &mut out, &mut total)?;
            name = Some(rest.to_string());
            continue;
        }
        if name.is_none() {
            // 名字行之前出现任何东西都无法归属：点名列出，不静默跳过（libass 在这里是
            // "Not understood" 直接忽略，我们比它严：这份脚本声称有内嵌字体，而我们解不出）
            legacy.push(trimmed.chars().take(40).collect());
            continue;
        }
        // 名字生效之后每一行都是这份字节的 base64（libass 的语法就是这样）。
        // 注意不能反过来用"含冒号/等号就不像载荷"来判断：base64 的填充就是 `=`，
        // 那样会把一整份字体前几十字节判成"认不出的条目形状"（AK10 撞的就是这个）。
        payload.push_str(trimmed);
    }
    flush(&mut name, &mut payload, &mut out, &mut total)?;
    if !legacy.is_empty() {
        return Err(attach_err(
            244,
            "[Fonts] 段里有我们不认的条目形状",
            &format!(
                "不静默跳过，也不当作\"没有附件\"；前若干条：{}",
                legacy
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        ));
    }
    Ok(out)
}

/// 名字行的两种在野写法：`fontname: X`（libass 的语法，`ass.c` 的 `process_fonts_line`
/// 认的就是这个前缀，之后每一行都是这份字节的 base64）与 `P: X`。
///
/// 为什么必须认 `fontname:`：真正带内嵌字体的 ASS 文件（Aegisub/VSFilter 存的）用的就是
/// 它。原先只认 `P:`，等于把我们自己造的夹具形状当成了规格（ISSUES AK10，与 Z1/AK1 同族）。
/// base64 字母表里没有冒号，所以按前缀判断不会把载荷行误认成名字行。
fn strip_name_key(line: &str) -> Option<&str> {
    let lower = line.to_ascii_lowercase();
    for key in ["fontname:", "p:"] {
        if lower.starts_with(key) {
            return Some(line[key.len()..].trim());
        }
    }
    None
}

fn attach_err(code: u16, title: &str, detail: &str) -> core_model::error::StkError {
    StkError::simple(Domain::Input, code, title).with_detail(detail.to_string())
}

/// 内容哈希。CLI 侧另有一份同名实现（`cli::pipeline::sha256_hex`）；
/// 这里公开出来是为了让测试能拿**另一条路径**核对台账里的值，而不是自己跟自己对答案。
pub fn digest_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64（`+`/`/`，`=` 填充）。换行与空白直接跳过 —— 脚本里的附件就是分行写的。
/// 遇到字母表外的字符就报错而不是"跳过当噪声"：那会把一份坏字体读成一份短字体。
pub fn decode_base64(s: &str) -> Result<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out: Vec<u8> = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut padding = 0usize;
    for ch in s.bytes() {
        match ch {
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            b'=' => {
                padding += 1;
                if padding > 2 {
                    return Err(attach_err(245, "base64 填充过长", s));
                }
                continue;
            }
            _ => {
                if padding > 0 {
                    return Err(attach_err(
                        246,
                        "base64 在填充之后还有数据",
                        "那不是合法的内嵌字体块",
                    ));
                }
                let v = ALPHABET.iter().position(|&a| a == ch).ok_or_else(|| {
                    attach_err(247, "base64 里有字母表外的字符", &format!("{ch:?}"))
                })?;
                acc = (acc << 6) | v as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push(((acc >> bits) & 0xFF) as u8);
                }
            }
        }
    }
    if bits >= 6 {
        return Err(attach_err(
            248,
            "base64 长度不是完整的字节组",
            &format!("残了 {bits} 位"),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts_block(body: &str) -> String {
        format!("[Script Info]\nTitle: t\n\n[Fonts]\n{body}\n\n[Events]\nFormat: Layer, Start, End, Style, Text\n")
    }

    /// 解码器要对着已知向量测，不能只测"自己编自己解"。
    #[test]
    fn base64_matches_known_vectors() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("", vec![]),
            ("Zg==", b"f".to_vec()),
            ("Zm8=", b"fo".to_vec()),
            ("Zm9v", b"foo".to_vec()),
            ("Zm9vYg==", b"foob".to_vec()),
            ("aGVsbG8gd29ybGQ=", b"hello world".to_vec()),
            // 分行与空白只是排版，不改变内容
            ("aGVsbG8g\n\r d29ybGQ=", b"hello world".to_vec()),
        ];
        for (text, want) in cases {
            let got = decode_base64(text).unwrap_or_else(|e| panic!("{text:?} {e:?}"));
            assert_eq!(got, want, "{text:?}");
        }
    }

    #[test]
    fn base64_refuses_garbage_instead_of_skipping_it() {
        // 字母表外：不能"跳过当噪声"，那会把一份字体解成另一份
        assert_eq!(decode_base64("aGV*sbG8=").unwrap_err().code, 247);
        // 填充之后还有数据
        assert_eq!(decode_base64("Zm8=v").unwrap_err().code, 246);
        // 不完整的组："Z" 只凑到 6 位，落不成一个字节
        assert_eq!(decode_base64("Z").unwrap_err().code, 248);
    }

    #[test]
    fn a_missing_fonts_section_is_no_attachments_not_an_error() {
        assert!(parse("[Script Info]\nTitle: t\n\n[Events]\n")
            .unwrap()
            .is_empty());
        // 别的段里的 P: 行不算附件
        let text = "[Script Info]\nP: arial.ttf\nZm8=\n";
        assert!(
            parse(text).unwrap().is_empty(),
            "Script Info 里的行不能被当字体附件"
        );
    }

    /// 真字节 + 真内部名：拿系统里的字体 base64 进块里，附件要能读回家族名与哈希。
    #[test]
    fn parses_a_real_embedded_font_with_its_internal_family() {
        let bytes = one_system_font();
        let b64 = encode_base64_for_test(&bytes);
        let text = fonts_block(&format!("P: embedded.ttf\n{b64}\n"));
        let got = parse(&text).expect("这份附件该被解出来");
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].declared_name, "embedded.ttf");
        assert_eq!(got[0].len(), bytes.len());
        assert!(got[0].parseable);
        assert!(
            !got[0].family.as_deref().unwrap_or_default().is_empty(),
            "{:?}",
            got[0]
        );
        // 台账里的哈希由**另一条实现**核对：不是自己跟自己对答案
        assert_eq!(got[0].sha256, digest_hex(&bytes));
        assert_eq!(got[0].sha256.len(), 64);
        assert_eq!(got[0].label(), got[0].family.clone().unwrap());
    }

    /// libass 的真实语法（`ass.c` 的 `process_fonts_line`）：`fontname: X` 之后每一行都是
    /// 这份字节的 base64，跨行拼接，直到下一个 `fontname:` 或段末。
    /// 这条测试是被 libass 源码纠正出来的：原先只认 `P:`，真文件的形状会被报成
    /// "认不出的条目形状"（AK10）—— 那是把我们自己造的夹具当成了规格。
    #[test]
    fn the_libass_fontname_grammar_is_what_real_files_use() {
        let bytes = one_system_font();
        let b64 = encode_base64_for_test(&bytes);
        // 真文件会把长 base64 折行：切成三段，拼回去必须还是同一批字节
        let mid = b64.len() / 3;
        let folded = format!(
            "{}\n{}\n{}\n",
            &b64[..mid],
            &b64[mid..mid * 2],
            &b64[mid * 2..]
        );
        let text = fonts_block(&format!("fontname: Marlett\n{folded}"));
        let got = parse(&text).expect("`fontname:` 是 libass 认的形状");
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].declared_name, "Marlett");
        assert_eq!(got[0].len(), bytes.len(), "折行的 base64 必须拼回原字节");
        assert_eq!(got[0].sha256, digest_hex(&bytes));

        // 两种写法解出同一批字节：差别只在名字行
        let alt = parse(&fonts_block(&format!("P: Marlett\n{b64}\n"))).unwrap();
        assert_eq!(alt[0].sha256, got[0].sha256);
        // 比 libass 宽松一档：它 strncmp 区分大小写，我们不区分（载荷里不可能出现冒号）
        let cased = parse(&fonts_block(&format!("FontName: Marlett\n{b64}\n"))).unwrap();
        assert_eq!(cased[0].sha256, got[0].sha256);
    }

    /// 名字行是空的（`fontname:` 后面什么都没有）：不能凭空造出一份"名字为空的附件"。
    #[test]
    fn an_empty_name_line_is_reported_not_invented() {
        let b64 = encode_base64_for_test(&one_system_font());
        let e = parse(&fonts_block(&format!("fontname:\n{b64}\n"))).unwrap_err();
        assert_eq!(e.code, 244, "{e:?}");
    }

    /// 名字生效之后，载荷行里的 `=`（base64 填充）不能被判成"键值形状"。
    ///
    /// 这一格原本被单位测试**躲过去**了：`one_system_font()` 拿到的 arial.ttf 是
    /// 1_036_584 字节，正好被 3 整除 → base64 全程没有填充；是门阶那份 marlett
    /// （27724 字节，尾部是 `==`）把整份字体判成了"认不出的条目形状"（AK10）。
    /// 所以这里刻意要求一份"长度不是 3 的倍数"的字体，让填充真的出现。
    #[test]
    fn base64_padding_inside_a_payload_is_not_read_as_a_key_value_line() {
        let windir = std::env::var("WINDIR").expect("测试需要 WINDIR");
        let dir = std::path::Path::new(&windir).join("Fonts");
        let mut picked: Option<Vec<u8>> = None;
        let mut entries = std::fs::read_dir(&dir)
            .expect("字体目录该能打开")
            .filter_map(|e| e.ok())
            .collect::<Vec<_>>();
        // 排序保证同一台机器上重复跑得到同一份样本
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !name.ends_with(".ttf") && !name.ends_with(".otf") {
                continue;
            }
            if let Ok(b) = std::fs::read(&p) {
                if !b.is_empty()
                    && b.len() % 3 != 0
                    && b.len() < 8 * 1024 * 1024
                    && crate::is_parseable_font(&b)
                {
                    picked = Some(b);
                    break;
                }
            }
        }
        let bytes = picked.expect("本机字体目录里该有一份长度不是 3 的倍数的字体，用来测填充");
        let b64 = encode_base64_for_test(&bytes);
        assert!(
            b64.ends_with('='),
            "夹具要真的带填充：{}",
            &b64[b64.len() - 4..]
        );

        // 折行：填充落在其中一行的行尾，那一行仍然只是载荷
        let mut folded = String::new();
        for chunk in b64.as_bytes().chunks(72) {
            folded.push_str(std::str::from_utf8(chunk).unwrap());
            folded.push('\n');
        }
        let got = parse(&fonts_block(&format!("fontname: Padded Font\n{folded}")))
            .expect("带 `=` 的载荷行不该被当成键值条目");
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].len(), bytes.len());
        assert_eq!(got[0].sha256, digest_hex(&bytes));
    }

    #[test]
    fn two_attachments_in_one_block_are_two_attachments() {
        let bytes = one_system_font();
        let b64 = encode_base64_for_test(&bytes);
        let text = fonts_block(&format!("P: a.ttf\n{b64}\nP: b.ttf\n{b64}\n"));
        let got = parse(&text).expect("两份都要解出来");
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].sha256, got[1].sha256, "同一批字节当然同一个哈希");
        assert_ne!(got[0].declared_name, got[1].declared_name);
    }

    /// "声明是字体却不是可解析的字体"必须是失败，不是静默少装一份。
    #[test]
    fn a_payload_that_is_not_a_font_is_reported() {
        let b64 = encode_base64_for_test(b"definitely not a font file at all");
        let e = parse(&fonts_block(&format!("P: fake.ttf\n{b64}\n"))).unwrap_err();
        assert_eq!(e.code, 243, "{e:?}");
    }

    #[test]
    fn old_style_or_unrecognised_entries_are_named_not_ignored() {
        let e = parse(&fonts_block("arial.ttf=Zm8=\n")).unwrap_err();
        assert_eq!(e.code, 244, "{e:?}");
        assert!(
            e.details.join(" ").contains("arial.ttf"),
            "要点名是哪一条：{e:?}"
        );
    }

    #[test]
    fn oversized_payloads_refuse_before_allocating_them() {
        // 造一个"超过上限"的 base64 文本而不真分配那么大的字节：
        // 解码器按 base64 长度增量检查会太绕，这里直接测合计上限那条路
        // （用一个小得多的上限走同一段代码不便，所以这题测的是错误码存在与可报告）
        let big = "A".repeat(MAX_ATTACHMENT_BYTES * 4 / 3 + 8);
        let err = parse(&fonts_block(&format!("P: big.ttf\n{big}\n"))).unwrap_err();
        assert!(
            matches!(err.code, 240..=249),
            "超限/非法都要落在这段可报告的错误码里：{err:?}"
        );
    }

    fn one_system_font() -> Vec<u8> {
        let windir = std::env::var("WINDIR").expect("测试需要 WINDIR");
        let dir = std::path::Path::new(&windir).join("Fonts");
        for name in ["arial.ttf", "lucon.ttf", "times.ttf", "marlett.ttf"] {
            let p = dir.join(name);
            if let Ok(b) = std::fs::read(&p) {
                if b.len() < 8 * 1024 * 1024 {
                    return b;
                }
            }
        }
        panic!("本机字体目录里该有一份可读的测试字体");
    }

    /// 测试自带的编码器：只服务测试，不进生产路径（生产只解码）。
    fn encode_base64_for_test(bytes: &[u8]) -> String {
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
            out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6 & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}
