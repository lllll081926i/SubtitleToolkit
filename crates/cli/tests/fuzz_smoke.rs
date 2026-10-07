//! 健壮性/模糊冒烟（28 §7 "fuzz smoke"、23 §健壮性、24 §QA）。
//!
//! 目标不是"找到 bug 就完"，而是钉住一条纪律：**任何字节序列都不许 panic**。
//! 崩溃的解析器会把诊断（22 §）变成一次进程退出，还会把 GUI 一起带走。
//!
//! 变异器是确定性的（自写 LCG，不引入 rand）：同一种子永远生成同一批输入，
//! 失败可以逐字重现，结果也不会随机器变化。

use std::path::{Path, PathBuf};

use codec_ass::ast::AssDocument;
use codec_pgs::encoder::{encode, EncodeOptions, Frame, FrameObject};
use codec_pgs::segment::write_stream;
use codec_srt::{parse as parse_srt, write as write_srt};

// ---------------------------------------------------------------- 确定性随机

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        // 数值配方：Knuth MMIX 的 LCG
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % (n as u64)) as usize
        }
    }
}

/// 单点变异。每一种都对应一种真实世界的损坏方式，而不是随手翻 bit。
fn mutate(bytes: &[u8], rng: &mut Lcg) -> Vec<u8> {
    let mut out = bytes.to_vec();
    if out.is_empty() {
        return out;
    }
    match rng.below(9) {
        0 => {
            let i = rng.below(out.len());
            let v = rng.next() as u8;
            out[i] ^= v; // bit 翻转
        }
        1 => {
            let len = rng.below(out.len() + 1);
            out.truncate(len); // 截断
        }
        2 => {
            let i = rng.below(out.len());
            out.insert(i, 0x00); // 内嵌 NUL
        }
        3 => {
            let i = rng.below(out.len());
            out.insert(i, b'\r'); // 行尾混乱
        }
        4 => {
            let i = rng.below(out.len());
            out.insert(i, 0xC3); // 半截 UTF-8：合法首字节 + 非法续字节
            out.insert(i + 1, 0x28);
        }
        5 => {
            let i = rng.below(out.len());
            let j = i + 1.min(out.len() - i);
            out.splice(i..j, [0xFFu8, 0xFE, 0x00]); // 垃圾块
        }
        6 => {
            // 复制一段：模拟重复事件 / 重复 segment
            let i = rng.below(out.len());
            let len = rng.below(40).min(out.len() - i);
            let chunk = out[i..i + len].to_vec();
            out.splice(i..i, chunk);
        }
        7 => out.clear(), // 空文件
        _ => {
            let i = rng.below(out.len());
            out[i] = b'\n';
        }
    }
    out
}

// ---------------------------------------------------------------- 语料

fn valid_ass() -> Vec<u8> {
    "[Script Info]\n\
Title: fuzz corpus\n\
ScriptType: v4.00+\n\
PlayResX: 1920\n\
PlayResY: 1080\n\
ScaledBorderAndShadow: yes\n\
\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,2,0,2,50,50,60,1\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\an5\\pos(960,540)}第一行{\\p1}m 0 0 l 10 10 l 10 20{\\p0}\n\
Dialogue: 0,0:00:04.00,0:00:06.00,Default,,0,0,0,,<i>斜体</i> 与 \\N 换行\n"
        .as_bytes()
        .to_vec()
}

fn valid_srt() -> Vec<u8> {
    "1\n00:00:01,000 --> 00:00:03,000\nhello\n\n2\n00:00:04,000 --> 00:00:06,000\n世界 with émojis 😀\n"
        .as_bytes()
        .to_vec()
}

fn valid_sup() -> Vec<u8> {
    let mut rgba = Vec::new();
    for _ in 0..32 {
        rgba.extend_from_slice(&[255, 255, 255, 255]);
    }
    let obj = FrameObject {
        rgba,
        width: 8,
        height: 4,
        x: 100,
        y: 900,
        forced: false,
    };
    let frames = [Frame::new(90_000, vec![obj]), Frame::erase(180_000)];
    let (segs, _) = encode(&frames, &EncodeOptions::default()).unwrap();
    write_stream(&segs).unwrap()
}

fn valid_bdn_xml() -> Vec<u8> {
    // 只作为"文本解析器不许 panic"的输入，不要求语义完全合法
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <BDNSubtitleData>\n\
     <Name>fuzz</Name>\n\
     <Language>zho</Language>\n\
     <VideoFormat>1080p25</VideoFormat>\n\
     <Delay>0</Delay>\n\
     <Style><Name>Default</Name><Font>Arial</Font><Size>64</Size><Colour>ff,ffffff</Colour></Style>\n\
     <Event><InTC>00:00:01:00</InTC><OutTC>00:00:03:00</OutTC>\n\
     <Command><Graphic>0001.png</Graphic><X>100</X><Y>200</Y><Width>800</Width><Height>40</Height></Command>\n\
     </Event>\n\
     </BDNSubtitleData>\n"
        .as_bytes()
        .to_vec()
}

// ---------------------------------------------------------------- 直接打解析器

#[test]
fn parsers_never_panic_on_mutated_bytes() {
    let corpora: Vec<(&str, Vec<u8>)> = vec![
        ("ass", valid_ass()),
        ("srt", valid_srt()),
        ("sup", valid_sup()),
        ("bdn-xml", valid_bdn_xml()),
    ];
    let mut checked = 0usize;
    for (ci, (name, base)) in corpora.iter().enumerate() {
        let mut rng = Lcg(2026_1006 + ci as u64);
        for _ in 0..600 {
            let bytes = mutate(base, &mut rng);
            checked += 1;
            match *name {
                "ass" => {
                    // codec-ass 是容忍式解析：只要求不 panic，不断言成功
                    let doc = AssDocument::parse(&String::from_utf8_lossy(&bytes));
                    let _ = doc.render();
                }
                "srt" => {
                    if let Ok((sub, _loss)) = parse_srt(&bytes) {
                        let _ = write_srt(&sub);
                    }
                }
                "sup" => {
                    let _ = codec_pgs::segment::parse_stream(&bytes);
                    let _ = codec_pgs::read_file(&bytes);
                }
                _ => {
                    let _ = codec_bdn::parse_bytes(&bytes);
                }
            }
        }
    }
    assert!(checked >= 2400, "变异样本太少不构成冒烟：{checked}");
}

#[test]
fn clean_inputs_are_idempotent_through_their_own_render() {
    // 性质检查：干净输入 render→parse→render 必须稳定；
    // 不稳定的话 golden 测试与"复制 CLI 命令重放"都会随机漂移
    let ass_text = String::from_utf8(valid_ass()).unwrap();
    let doc = AssDocument::parse(&ass_text);
    let once = doc.render();
    let twice = AssDocument::parse(&once).render();
    assert_eq!(once, twice, "ASS render 必须幂等");
    assert!(once.contains("fuzz corpus"));

    let srt = valid_srt();
    let (sub, _loss) = parse_srt(&srt).unwrap();
    let one = write_srt(&sub).unwrap();
    let (sub2, _) = parse_srt(&one).unwrap();
    let two = write_srt(&sub2).unwrap();
    assert_eq!(one, two, "SRT 往返必须字节稳定");

    let sup = valid_sup();
    let segs = codec_pgs::segment::parse_stream(&sup).unwrap();
    let again = write_stream(&segs).unwrap();
    assert_eq!(
        sup, again,
        "已解析的 SUP 重写必须与原字节相同（Fast Path 的前提）"
    );
}

// ---------------------------------------------------------------- 打真实入口

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("stk-fuzz-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::create_dir_all(&p);
    p
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn cli_probe_entry_never_panics_and_always_explains_itself() {
    // probe 的契约是"告诉我你看到了什么"（20 §Commands、16）：
    // 它不会因为内容看不懂就失败，但必须把判定与置信度交代清楚。
    let dir = tmpdir("probe");
    let mut rng = Lcg(777);
    let base = valid_ass();
    let mut unknown = 0usize;
    for i in 0..300 {
        let bytes = mutate(&base, &mut rng);
        let path = write(&dir, &format!("m{i:04}.ass"), &bytes);
        let argv = vec![
            "probe".to_string(),
            path.to_string_lossy().to_string(),
            "--json".to_string(),
        ];
        let inv = cli::args::parse(&argv).expect("probe 参数应合法");
        let env = cli::run::execute(&inv).expect("probe 对任意内容都应给出可交代的结果");
        let line = env.to_json_line();
        assert!(!line.contains('\n'), "信封必须是单行：{line}");
        let v: serde_json::Value = serde_json::from_str(&line).expect("信封必须是合法 JSON");
        assert!(v["schema"].is_number(), "{line}");
        assert_eq!(v["tool"], "subtitletool-cli");
        assert!(v["ok"].is_boolean(), "{line}");
        let det = &v["data"]["detection"];
        assert!(det["format"].is_string(), "probe 必须报告格式判定：{line}");
        assert!(
            det["confidence"].is_string(),
            "probe 必须报告置信度：{line}"
        );
        assert!(
            det["reasons"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "probe 必须给出理由，否则用户无法复核判定：{line}"
        );
        if det["format"] == serde_json::Value::String("unknown".into()) {
            unknown += 1;
            // 认不出来就不许假装高置信
            assert_eq!(
                det["confidence"], "Low",
                "unknown 却给了非 Low 置信：{line}"
            );
        }
    }
    assert!(unknown > 0, "变异器没能造出认不出的输入：{unknown}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn printf_hint(failed: usize, succeeded: usize) {
    // 让比例在测试输出里可见：只有"失败一堆"或"成功一堆"都值得再看一眼
    eprintln!("convert 变异结果：failed={failed} succeeded={succeeded}");
}

#[test]
fn cli_convert_entry_fails_cleanly_on_garbage_and_never_writes_a_bad_file() {
    // convert 才是"要动手"的命令：看不懂就必须失败，且不能留下半成品（38 §1）
    let dir = tmpdir("convert");
    let mut rng = Lcg(4242);
    let base = valid_srt();
    let mut failed = 0usize;
    let mut succeeded = 0usize;
    for i in 0..200 {
        let bytes = mutate(&base, &mut rng);
        let src = write(&dir, &format!("g{i:04}.srt"), &bytes);
        let out = dir.join(format!("o{i:04}.ass"));
        let argv = vec![
            "convert".to_string(),
            src.to_string_lossy().to_string(),
            "--to".to_string(),
            "ass".to_string(),
            "--out".to_string(),
            out.to_string_lossy().to_string(),
            "--json".to_string(),
        ];
        let inv = cli::args::parse(&argv).expect("convert 参数应合法");
        match cli::run::execute(&inv) {
            Ok(env) => {
                succeeded += 1;
                let line = env.to_json_line();
                assert!(!line.contains('\n'));
                let v: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(v["ok"], true, "成功信封 ok 必须为 true：{line}");
                // 真写了文件就必须能被再次解析（不许产出自己读不了的东西）
                if out.exists() {
                    let back = std::fs::read(&out).unwrap();
                    let _ = codec_ass::ast::AssDocument::parse(&String::from_utf8_lossy(&back));
                }
            }
            Err(_) => {
                failed += 1;
                assert!(
                    !out.exists(),
                    "失败却留下了输出文件（38 §1）：{}",
                    out.display()
                );
            }
        }
    }
    printf_hint(failed, succeeded);
    assert!(
        failed > 0,
        "200 个变异输入没有一个失败，说明 convert 对垃圾太宽容"
    );
    assert!(
        succeeded > 0,
        "没有一个输入转换成功，说明变异器只会造垃圾，覆盖不到「可读」分支"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
