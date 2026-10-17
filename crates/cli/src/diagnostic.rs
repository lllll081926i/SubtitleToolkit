//! 22 §5 的诊断包：把一次故障现场打成一个 zip，交给支持人员。
//!
//! ## 规格里最要紧的一句
//!
//! 「**默认不包含原字幕/视频**；用户显式选择才附样本。」
//!
//! 这条不是"顺手为之"的隐私声明，而是这个功能的**设计约束**：诊断包天生是
//! 要被发出去的（贴到 issue、发邮件），所以默认内容必须是可以直接外发的。
//! 任何"反正先收进来，发之前再删"的做法都会在某个人身上失效 ——
//! 那个人恰好是没读说明、直接拖进邮件窗口的用户。
//!
//! 因此这里的设计是：
//!
//! 1. **默认白名单**：只收版本、平台、设置（脱敏后）、依赖身份、命令与错误摘要。
//!    这些没有一样来自用户的字幕内容。
//! 2. **样本要显式点名**：`--include-input` 才附，且附的时候**在包里写明**
//!    「这里含用户内容」—— 收包的人不必去猜。
//! 3. **敏感值按名字脱敏**：22 §5 写的是 settings-redacted。脱敏规则放在
//!    [`redact_settings`]，判据是"键名像不像敏感项"，而不是"值像不像"——
//!    值可能是任何东西，键名是有限且可枚举的。
//!
//! ## 为什么这里手写 ZIP 而不是引一个 zip 库
//!
//! `flate2` 已经在依赖树里（`image-core` 的 PNG 用它），而 ZIP 的
//! **stored/deflate + 中央目录**这部分格式是稳定的、有公开规范的、
//! 且我们只需要写不需要读。为这一个用途引入一个 zip crate，
//! 会多一条需要审计来源与许可的依赖（21 §11），收益不成比例。
//! 压缩本身仍然交给 flate2 —— 与 PNG 那条"不自己实现 deflate"的理由一致。
//!
//! 但"手写格式"意味着**必须自己验**：`read_back_entries` 是本文件里的
//! 独立读取器，只用于测试，用来证明写出的字节是**真能被解开的 zip**，
//! 而不是一个长得像 zip 的东西。

use std::io::Write;
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Output, code, msg)
}

/// 一个条目的内容来源。
pub enum Entry {
    /// 文本（UTF-8，作为文件内容）
    Text(String),
    /// 磁盘上的文件
    File(PathBuf),
}

/// 要写进包里的一个条目。
pub struct Item {
    /// 包内路径，用 `/` 分隔。必须是相对路径（不许 `..` 或绝对路径）。
    pub name: String,
    pub body: Entry,
    /// 是否为"用户内容"。为真时报告会单独列出，让人知道发了什么出去。
    pub user_content: bool,
}

/// 组装好的诊断包。
pub struct Bundle {
    items: Vec<Item>,
}

impl Bundle {
    pub fn new() -> Bundle {
        Bundle { items: Vec::new() }
    }

    /// 加一条文本。`name` 是包内路径。
    pub fn text(&mut self, name: &str, body: impl Into<String>) -> &mut Self {
        self.items.push(Item {
            name: name.to_string(),
            body: Entry::Text(body.into()),
            user_content: false,
        });
        self
    }

    /// 标成"用户内容"的文本（例如用户显式附上的输入文件清单）。
    pub fn user_text(&mut self, name: &str, body: impl Into<String>) -> &mut Self {
        self.items.push(Item {
            name: name.to_string(),
            body: Entry::Text(body.into()),
            user_content: true,
        });
        self
    }

    /// 附一个磁盘文件。`name` 为 `None` 时用文件名。
    ///
    /// 读不到文件时**不失败**：诊断包的意义在于"在出问题的机器上尽量多收证据"，
    /// 而一个读不到的文件（权限、已被删）本身也是证据。这里记一条说明代替内容。
    pub fn file(&mut self, name: Option<&str>, path: &Path, user_content: bool) -> &mut Self {
        let resolved = name
            .map(|s| s.to_string())
            .or_else(|| path.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| "unnamed".to_string());
        let body = match std::fs::read(path) {
            Ok(b) => Entry::Text(String::from_utf8_lossy(&b).to_string()),
            Err(e) => Entry::Text(format!("(诊断包无法读取该文件：{}：{e})\n", path.display())),
        };
        self.items.push(Item {
            name: resolved,
            body,
            user_content,
        });
        self
    }

    /// 包内的条目名（报告与测试用）。
    pub fn names(&self) -> Vec<&str> {
        self.items.iter().map(|i| i.name.as_str()).collect()
    }

    /// 声明含用户内容的条目名。
    pub fn user_content_names(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter(|i| i.user_content)
            .map(|i| i.name.as_str())
            .collect()
    }

    /// 打成 zip 字节。
    pub fn to_zip(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for it in &self.items {
            let name = it.name.as_bytes();
            check_name(&it.name)?;
            let data = match &it.body {
                Entry::Text(t) => t.as_bytes().to_vec(),
                Entry::File(p) => std::fs::read(p).map_err(|e| {
                    cli_err(400, "诊断包：读不到条目文件")
                        .with_detail(format!("{}：{e}", p.display()))
                })?,
            };
            let crc = crc32(&data);
            let deflated = deflate(&data)?;
            // deflate 之后更大就用 stored：小文本上这是常态，不这么做会得到更大的包。
            let (method, payload) = if deflated.len() < data.len() {
                (8u16, deflated)
            } else {
                (0u16, data.clone())
            };
            let offset = out.len() as u32;
            let local = local_header(name, method, crc, payload.len() as u32, data.len() as u32);
            out.extend_from_slice(&local);
            out.extend_from_slice(name);
            out.extend_from_slice(&payload);
            // 中央目录项也是「固定头 + 名字」两段。本地头那边写了名字，
            // 这边漏写就会得到一个"本地头带着名字、中央目录没带"的包：
            // 它开头的签名全都对，只是 EOCD 里的 cd_size 偏小 —— Python 的 zipfile
            // 报 `Truncated central directory`，而自家读取器在第二项才报签名不符。
            let chead = central_header(
                name,
                method,
                crc,
                payload.len() as u32,
                data.len() as u32,
                offset,
            );
            central.extend_from_slice(&chead);
            central.extend_from_slice(name);
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        let eocd =
            end_of_central_directory(self.items.len() as u16, central.len() as u32, cd_offset);
        out.extend_from_slice(&eocd);
        Ok(out)
    }
}

impl Default for Bundle {
    fn default() -> Self {
        Bundle::new()
    }
}

/// 条目名必须是相对路径且不带 `..`。
///
/// 包会被解到某个目录里，而 zip 的路径串是攻击面之一（zip-slip：`../../x` 逃出目标目录）。
/// 我们自己生成的条目名应当都是安全的，但**生成侧的安全假设不该成为唯一防线** ——
/// 加一道显式检查，`--include-input` 带上用户提供的文件名时它才真的在防东西。
fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains(':') && name.len() > 1 && name.as_bytes()[1] == b':'
        || name.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err(cli_err(401, "诊断包：条目名不是安全的相对路径").with_detail(name.to_string()));
    }
    Ok(())
}

// --- 格式细节 ---------------------------------------------------------------

fn deflate(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::{write::DeflateEncoder, Compression};
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data)
        .map_err(|err| cli_err(402, "诊断包：压缩失败").with_detail(err.to_string()))?;
    e.finish()
        .map_err(|err| cli_err(403, "诊断包：压缩收尾失败").with_detail(err.to_string()))
}

/// CRC-32（ZIP 用的标准多项式 0xEDB88320）。查表比逐位算快得多，而这张表只有 1 KiB。
fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn local_header(name: &[u8], method: u16, crc: u32, csize: u32, usize_: u32) -> Vec<u8> {
    // 本地文件头（ZIP 规范 4.3.7）：固定 30 字节。
    // 字段顺序与宽度就是下面这 11 行的顺序；任何一行多写/少写一个字段，
    // EOCD 里声明的中央目录偏移就会整偏，而**包本身依然像 zip**。
    let mut h = Vec::with_capacity(30);
    h.extend_from_slice(&0x0403_4B50u32.to_le_bytes()); // 0 签名
    h.extend_from_slice(&20u16.to_le_bytes()); // 4 解压所需版本
    h.extend_from_slice(&0u16.to_le_bytes()); // 6 通用标志（无加密、非流式）
    h.extend_from_slice(&method.to_le_bytes()); // 8 压缩方法
    h.extend_from_slice(&0u16.to_le_bytes()); // 10 修改时间：固定 0
    h.extend_from_slice(&0x21u16.to_le_bytes()); // 12 修改日期：固定 1980-01-01
    h.extend_from_slice(&crc.to_le_bytes()); // 14 CRC-32
    h.extend_from_slice(&csize.to_le_bytes()); // 18 压缩后大小
    h.extend_from_slice(&usize_.to_le_bytes()); // 22 原大小
    h.extend_from_slice(&(name.len() as u16).to_le_bytes()); // 26 名字长度
    h.extend_from_slice(&0u16.to_le_bytes()); // 28 扩展区长度
    debug_assert_eq!(h.len(), 30, "本地头固定部分必须正好 30 字节");
    h
}

fn central_header(
    name: &[u8],
    method: u16,
    crc: u32,
    csize: u32,
    usize_: u32,
    offset: u32,
) -> Vec<u8> {
    let mut h = Vec::with_capacity(46 + name.len());
    h.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
    h.extend_from_slice(&20u16.to_le_bytes()); // 创建版本
    h.extend_from_slice(&20u16.to_le_bytes()); // 解压所需版本
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&method.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&0x21u16.to_le_bytes());
    h.extend_from_slice(&crc.to_le_bytes());
    h.extend_from_slice(&csize.to_le_bytes());
    h.extend_from_slice(&usize_.to_le_bytes());
    h.extend_from_slice(&(name.len() as u16).to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // 扩展区长度
    h.extend_from_slice(&0u16.to_le_bytes()); // 注释长度
    h.extend_from_slice(&0u16.to_le_bytes()); // 起始磁盘号
    h.extend_from_slice(&0u16.to_le_bytes()); // 内部属性（文本文件）
    h.extend_from_slice(&0u32.to_le_bytes()); // 外部属性
    h.extend_from_slice(&offset.to_le_bytes());
    // 固定部分恰好 46 字节（名字放在项**之后**）。
    debug_assert_eq!(h.len(), 46, "中央目录项固定部分必须正好 46 字节");
    h
}

fn end_of_central_directory(count: u16, cd_size: u32, cd_offset: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(22);
    h.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // 本磁盘号
    h.extend_from_slice(&0u16.to_le_bytes()); // 中央目录起始磁盘
    h.extend_from_slice(&count.to_le_bytes()); // 本磁盘条目数
    h.extend_from_slice(&count.to_le_bytes()); // 总条目数
    h.extend_from_slice(&cd_size.to_le_bytes());
    h.extend_from_slice(&cd_offset.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // 注释长度
    h
}

/// 22 §5：settings-redacted。
///
/// 判据是**键名**而不是值：值的形态无穷无尽，键名是有限且可枚举的。
/// 脱敏后仍然保留 `键=⟨已脱敏⟩` 的形状而不是删掉整行 —— "这个设置存在但没给你看"
/// 与"没有这个设置"是两件不同的事，支持人员需要能区分。
pub fn redact_settings(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, val)| {
                    let lower = k.to_ascii_lowercase();
                    let sensitive = [
                        "token",
                        "secret",
                        "password",
                        "passwd",
                        "key",
                        "credential",
                        "auth",
                        "cookie",
                        "license",
                        "serial",
                        "user",
                        "email",
                        "path",
                        "dir",
                    ]
                    .iter()
                    .any(|s| lower.contains(s));
                    if sensitive {
                        (k.clone(), serde_json::json!("⟨已脱敏⟩"))
                    } else {
                        (k.clone(), redact_settings(val))
                    }
                })
                .collect(),
        ),
        // 非对象原样返回：脱敏只改"哪一个键"的可见性，不改值的形状。
        // 早期版本对数组/标量一律返回空对象，会把 `default_canvas: "1920x1080"`
        // 这类正常值也变成 `{}` —— 脱敏变成了破坏。
        other => other.clone(),
    }
}

/// 平台与版本摘要。诊断包里这一条的价值最高：它决定"能不能复现"。
pub fn environment(app_version: &str) -> serde_json::Value {
    let dir = core_model::DataDir::resolve_current_exe();
    serde_json::json!({
        "app_version": app_version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "exe_suffix": std::env::consts::EXE_SUFFIX,
        "pid": std::process::id(),
        "data_dir": {
            "path": dir.path.as_ref().map(|p| p.display().to_string()),
            "kind": dir.kind.label(),
            "why": dir.why,
        },
        // 22 §1 说日志落在数据目录；这里报出来，让人知道该去哪儿找
        "logs_dir": dir.file("logs").map(|p| p.display().to_string()),
    })
}

/// 生产用的回读校验：确认写出的字节是一个结构完整、条目数与声明一致的 zip。
///
/// 与 `pipeline::emit` 的其它调用点同一条纪律：**能写不能读的产物不算写成功**。
/// 这里不刻意复用测试里那个读取器：测试版故意做得更严（逐条解压 + 验 CRC），
/// 生产版只做能在毫秒内跑完的检查。两者都要，但目的不同。
///
/// 为什么这条检查不是多余的：手写 ZIP 最容易错的地方就是 EOCD 里的
/// `cd_size`/`cd_offset` 与条目数，而错了之后**包开头的签名全都对** ——
/// 只有在真去解的时候才报 `Truncated central directory`。
pub fn verify_zip(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 22 {
        return Err(cli_err(410, "诊断包：字节太少，不可能是合法 zip"));
    }
    let eocd = bytes.len() - 22;
    if u32::from_le_bytes(bytes[eocd..eocd + 4].try_into().unwrap()) != 0x0605_4B50 {
        return Err(cli_err(411, "诊断包：结尾没有 EOCD 签名"));
    }
    let count = u16::from_le_bytes(bytes[eocd + 10..eocd + 12].try_into().unwrap()) as usize;
    let cd_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    let cd_off = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    if count == 0 {
        // 空包是合法的（只有 EOCD）
        return (bytes.len() == 22)
            .then_some(())
            .ok_or_else(|| cli_err(412, "诊断包：声明 0 个条目却有多余字节"));
    }
    if cd_off + cd_size != eocd {
        return Err(cli_err(413, "诊断包：EOCD 声明的中央目录位置与实际不符")
            .with_detail(format!("cd_off={cd_off} cd_size={cd_size} 相加应为 {eocd}")));
    }
    // 逐条走一遍，确认每条的长度字段自洽（只看结构，不解压）
    let mut at = cd_off;
    for i in 0..count {
        if at + 46 > bytes.len() {
            return Err(cli_err(414, "诊断包：中央目录被截断").with_detail(format!("第 {i} 项")));
        }
        if u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) != 0x0201_4B50 {
            return Err(
                cli_err(415, "诊断包：中央目录项签名不符").with_detail(format!("第 {i} 项 @{at}"))
            );
        }
        let nlen = u16::from_le_bytes(bytes[at + 28..at + 30].try_into().unwrap()) as usize;
        let elen = u16::from_le_bytes(bytes[at + 30..at + 32].try_into().unwrap()) as usize;
        let clen = u16::from_le_bytes(bytes[at + 32..at + 34].try_into().unwrap()) as usize;
        at += 46 + nlen + elen + clen;
    }
    if at != eocd {
        return Err(
            cli_err(416, "诊断包：走完声明的条目数后位置与中央目录结尾不符")
                .with_detail(format!("走完后 {at}，应为 {eocd}")),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立读取器：只做测试用，用来证明写出的字节是**真 zip**。
    ///
    /// 它不复用写入侧的任何代码（除了 CRC 与 inflate 这两个外部事实），
    /// 所以"写读自洽"不会自证。这是本文件里最重要的测试支撑。
    fn read_back_entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        // 从尾部找 EOCD（注释为空时就在最后 22 字节）
        let eocd = zip.len() - 22;
        assert_eq!(
            u32::from_le_bytes(zip[eocd..eocd + 4].try_into().unwrap()),
            0x0605_4B50,
            "找不到 EOCD 签名"
        );
        let count = u16::from_le_bytes(zip[eocd + 10..eocd + 12].try_into().unwrap()) as usize;
        let cd_off = u32::from_le_bytes(zip[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        // 空包没有中央目录，偏移就是 0 —— 这是合法 zip 的形状，不是错误。
        if count == 0 {
            return Vec::new();
        }
        assert!(
            cd_off + 4 <= zip.len(),
            "EOCD 声明的中央目录偏移 {cd_off} 超出包长 {}",
            zip.len()
        );
        assert_eq!(
            u32::from_le_bytes(zip[cd_off..cd_off + 4].try_into().unwrap()),
            0x0201_4B50,
            "EOCD 声明的偏移 {cd_off} 处不是中央目录签名（包长 {}）",
            zip.len()
        );
        let mut at = cd_off;
        let mut out = Vec::new();
        for _ in 0..count {
            assert_eq!(
                u32::from_le_bytes(zip[at..at + 4].try_into().unwrap()),
                0x0201_4B50,
                "中央目录项签名不对 @{at}"
            );
            let method = u16::from_le_bytes(zip[at + 10..at + 12].try_into().unwrap());
            let crc = u32::from_le_bytes(zip[at + 16..at + 20].try_into().unwrap());
            let csize = u32::from_le_bytes(zip[at + 20..at + 24].try_into().unwrap()) as usize;
            let usize_ = u32::from_le_bytes(zip[at + 24..at + 28].try_into().unwrap()) as usize;
            let nlen = u16::from_le_bytes(zip[at + 28..at + 30].try_into().unwrap()) as usize;
            let elen = u16::from_le_bytes(zip[at + 30..at + 32].try_into().unwrap()) as usize;
            let clen = u16::from_le_bytes(zip[at + 32..at + 34].try_into().unwrap()) as usize;
            let lho = u32::from_le_bytes(zip[at + 42..at + 46].try_into().unwrap()) as usize;
            let name =
                String::from_utf8(zip[at + 46..at + 46 + nlen].to_vec()).expect("条目名不是 UTF-8");
            // 本地头
            assert_eq!(
                u32::from_le_bytes(zip[lho..lho + 4].try_into().unwrap()),
                0x0403_4B50,
                "本地头签名不对 @{lho} ({name})"
            );
            let l_nlen = u16::from_le_bytes(zip[lho + 26..lho + 28].try_into().unwrap()) as usize;
            let l_elen = u16::from_le_bytes(zip[lho + 28..lho + 30].try_into().unwrap()) as usize;
            let data_at = lho + 30 + l_nlen + l_elen;
            let payload = &zip[data_at..data_at + csize];
            let data = match method {
                0 => payload.to_vec(),
                8 => {
                    use flate2::read::DeflateDecoder;
                    use std::io::Read;
                    let mut d = DeflateDecoder::new(payload);
                    let mut buf = Vec::new();
                    d.read_to_end(&mut buf).expect("deflate 解不开");
                    buf
                }
                m => panic!("不支持的方法 {m}"),
            };
            assert_eq!(data.len(), usize_, "解出的长度与头部声明不符（{name}）");
            assert_eq!(crc32(&data), crc, "CRC 不符（{name}）");
            out.push((name, data));
            at += 46 + nlen + elen + clen;
        }
        out
    }

    #[test]
    fn the_written_bytes_are_a_real_zip_that_an_independent_reader_can_open() {
        let mut b = Bundle::new();
        b.text("readme.txt", "hello 诊断\n")
            .text("nested/deep.json", r#"{"a":1}"#);
        let zip = b.to_zip().unwrap();
        assert_eq!(&zip[..4], b"PK\x03\x04", "开头该是本地头签名");
        // 生产用的回读校验也必须通过 —— 否则它会卡住每一次 diagnose
        verify_zip(&zip).expect("verify_zip 该通过");
        let got = read_back_entries(&zip);
        assert_eq!(
            got.len(),
            2,
            "{:?}",
            got.iter().map(|x| &x.0).collect::<Vec<_>>()
        );
        assert_eq!(got[0].0, "readme.txt");
        assert_eq!(got[0].1, "hello 诊断\n".as_bytes());
        assert_eq!(got[1].0, "nested/deep.json");
        assert_eq!(got[1].1, br#"{"a":1}"#);
    }

    /// 空包也得是个合法 zip（EOCD 只有 22 字节）。
    #[test]
    fn an_empty_bundle_is_still_a_valid_zip() {
        let zip = Bundle::new().to_zip().unwrap();
        assert_eq!(zip.len(), 22);
        verify_zip(&zip).expect("空包该通过 verify_zip");
        assert!(read_back_entries(&zip).is_empty());
    }

    /// `verify_zip` 必须真的能看出问题。
    ///
    /// 少了这条负基线，一个"什么都返回 Ok"的实现也能让上面所有用例变绿，
    /// 而它在生产里干的是把坏包当成功交出去。
    #[test]
    fn verify_zip_rejects_deliberately_broken_bundles() {
        let mut b = Bundle::new();
        b.text("a.txt", "x").text("b.txt", "y");
        let good = b.to_zip().unwrap();
        verify_zip(&good).expect("好包该过");

        // 1. 太短
        assert!(verify_zip(&[]).is_err());
        assert!(verify_zip(&good[..10]).is_err());

        // 2. 切掉尾巴（EOCD 就没了）
        assert!(verify_zip(&good[..good.len() - 5]).is_err());

        // 3. EOCD 里的条目数改大 → 走条目时会越界
        let mut v = good.clone();
        let eocd = v.len() - 22;
        v[eocd + 10..eocd + 12].copy_from_slice(&9u16.to_le_bytes());
        assert!(verify_zip(&v).is_err(), "条目数被改大该被查出来");

        // 4. EOCD 里的中央目录偏移改错
        let mut v = good.clone();
        let eocd = v.len() - 22;
        v[eocd + 16..eocd + 20].copy_from_slice(&7u32.to_le_bytes());
        assert!(verify_zip(&v).is_err(), "偏移被改错该被查出来");

        // 5. 中央目录项签名被破坏
        let mut v = good.clone();
        let eocd = v.len() - 22;
        let cd = u32::from_le_bytes(v[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        v[cd] = 0;
        assert!(verify_zip(&v).is_err(), "中央目录签名被破坏该被查出来");
    }

    /// 大且不可压缩的内容走 stored 分支，仍要能读回来。
    #[test]
    fn incompressible_content_falls_back_to_stored_and_still_roundtrips() {
        let mut b = Bundle::new();
        // 伪随机：deflate 压不动
        let data: String = (0..2000u32)
            .map(|i| char::from_u32(0x4E00 + (i * 7919) % 20000).unwrap())
            .collect();
        b.text("big.txt", data.clone());
        let zip = b.to_zip().unwrap();
        let got = read_back_entries(&zip);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, data.as_bytes());
    }

    /// zip-slip：条目名不许逃出目标目录。
    #[test]
    fn a_path_traversal_entry_name_is_refused() {
        for bad in ["../escape.txt", "a/../../b", "/abs", "C:/x", ""] {
            let mut b = Bundle::new();
            b.text(bad, "x");
            assert!(b.to_zip().is_err(), "{bad:?} 该被拒");
        }
        // 正常名字仍要通过（否则上面的测试可能只是"什么都不接受"）
        let mut ok = Bundle::new();
        ok.text("logs/app.log", "x");
        assert!(ok.to_zip().is_ok());
    }

    /// 脱敏按**键名**判定，并保留"这个键存在"这个事实。
    #[test]
    fn redaction_hides_sensitive_keys_but_keeps_their_presence() {
        let v = serde_json::json!({
            "log_level": "debug",
            "api_token": "abcd1234",
            "nested": {"user_email": "a@b.c", "keep": 1},
            "default_canvas": "1920x1080"
        });
        let r = redact_settings(&v);
        assert_eq!(r["log_level"], "debug", "非敏感项该原样保留");
        assert_eq!(r["default_canvas"], "1920x1080");
        assert_eq!(r["api_token"], "⟨已脱敏⟩");
        assert!(r.get("api_token").is_some(), "该保留键的存在");
        assert_eq!(r["nested"]["user_email"], "⟨已脱敏⟩");
        assert_eq!(r["nested"]["keep"], 1);
    }

    /// 用户内容必须被标出来 —— 这是"默认不外发用户内容"的判据。
    #[test]
    fn user_content_is_marked_so_it_can_be_reported() {
        let mut b = Bundle::new();
        b.text("environment.json", "{}")
            .user_text("input/subtitle.ass", "[Script Info]\n");
        assert_eq!(b.user_content_names(), vec!["input/subtitle.ass"]);
        assert!(b.names().contains(&"environment.json"));
    }

    /// 读不到的文件不该让整包失败（在出问题的机器上收集证据时这是常态）。
    #[test]
    fn an_unreadable_file_records_a_note_instead_of_failing() {
        let mut b = Bundle::new();
        let ghost = std::env::temp_dir().join("stk-definitely-missing-9f8a7b6c5d");
        b.file(Some("missing.log"), &ghost, false);
        let zip = b.to_zip().unwrap();
        let got = read_back_entries(&zip);
        assert_eq!(got.len(), 1);
        let text = String::from_utf8_lossy(&got[0].1);
        assert!(text.contains("无法读取"), "{text}");
        assert!(text.contains("stk-definitely-missing"), "{text}");
    }
}
