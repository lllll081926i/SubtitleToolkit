//! 错误码的**跨 crate 身份唯一性**（22 §错误模型、ISSUES H8）。
//!
//! 一个 `(domain, code)` 是对外稳定的标识：报告、日志、手册、测试断言都靠它。
//! 同一棵树里两个 crate 各自用同一个编号表达两件不同的事，读报告的人就只能靠标题猜 ——
//! 而"撞了码"这件事没有任何机器把关，改代码的人不会被告知。这条测试是那层把关。
//!
//! 刻意只管**跨 crate** 的撞码：同一个 crate 内用一个编号承载一族相关错误
//! （例如 Bdn/56 下几个属性值非法，具体属性写在 details 里）是本仓库既有的风格，
//! 那是另一种口径要不要收紧的产品决定（见 ISSUES H8 的第二段），不是这里能替它判的。
//! 跨 crate 时规则很硬：要么编号不同，要么标题逐字相同（= 真的同一件事）。
//!
//! 扫的是两种写法，缺一种就是假绿：
//! 1. 直接点：`StkError::simple(Domain::Pgs, 1, "PTS 为负")`，编号也可能是命名常量
//!    （`codes::OVERFLOW`），常量表按 crate 汇总（常量常和使用点不在同一个文件）。
//! 2. 转发点：`fn cli_err(code: u16, msg: &str)` + 调用 `cli_err(150, "…")`。
//!    CLI 的整段 110–159、image-core、codec-bdn、pgs-author、font-service 的附件码都走这条路，
//!    只扫 `Domain::` 会把它们全漏掉 —— 而 font-service 和 cli 用的是**同一个** `Domain::Input`。
//!
//! 注释与字符串字面量的内容会被掩掉，所以文档或 `format!("…")` 里出现的 `Domain::Pgs, 1`
//! 不会被当成一个真的错误码点。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// (domain, code) -> { (crate, 标题) -> 文件 }
type Seen = BTreeMap<(String, u16), BTreeMap<(String, String), Vec<String>>>;

/// 本文件里的一个转发 helper：`fn err(code: u16, title: &str)`。
#[derive(Clone)]
struct Trampoline {
    /// `code` 形参的名字与位置
    code_param: String,
    code_index: usize,
    title_index: usize,
    /// 编号形参所在的语句块（用于区分"定义/转发"和"真正的调用点"）
    body: (usize, usize),
    /// 认不出 domain 之前是 None
    domain: Option<String>,
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let skip = p
                .file_name()
                .map(|n| n == "target" || n == ".git")
                .unwrap_or(false);
            if !skip {
                collect_rs(&p, out);
            }
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// 测试代码里的编号是给断言用的样例，不构成对外契约：`#[cfg(test)]` 之后的部分跳过。
fn production_only(src: &str) -> &str {
    match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// 掩掉行注释、块注释、字符串与字符字面量的**内容**（长度不变，位置一一对应）。
/// 后面所有"找关键词/配对括号"的判断都只看不被掩的位置。
fn mask(src: &str) -> Vec<bool> {
    let s: Vec<char> = src.chars().collect();
    let mut m = vec![false; s.len()];
    // 只掩内容、不掩定界符：后面的字面量识别要靠那对引号认标题。
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == '/' && i + 1 < s.len() && s[i + 1] == '/' {
            i += 2;
            while i < s.len() && s[i] != '\n' {
                m[i] = true;
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < s.len() && s[i + 1] == '*' {
            let mut depth = 1; // Rust 的块注释可嵌套
            i += 2;
            while i < s.len() && depth > 0 {
                if s[i] == '/' && i + 1 < s.len() && s[i + 1] == '*' {
                    depth += 1;
                    m[i] = true;
                    m[i + 1] = true;
                    i += 2;
                    continue;
                }
                if s[i] == '*' && i + 1 < s.len() && s[i + 1] == '/' {
                    depth -= 1;
                    m[i] = true;
                    m[i + 1] = true;
                    i += 2;
                    continue;
                }
                m[i] = true;
                i += 1;
            }
            continue;
        }
        // 原始字符串 r"…" / r#"…"#
        if c == 'r' && i + 1 < s.len() && (s[i + 1] == '"' || s[i + 1] == '#') {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < s.len() && s[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < s.len() && s[j] == '"' {
                i = j + 1;
                let closing: Vec<char> = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                while i < s.len() {
                    if s[i] == '"' && s[i..].starts_with(&closing) {
                        i += closing.len();
                        break;
                    }
                    m[i] = true;
                    i += 1;
                }
                continue;
            }
        }
        if c == '"' {
            i += 1;
            while i < s.len() {
                if s[i] == '\\' {
                    m[i] = true;
                    if i + 1 < s.len() {
                        m[i + 1] = true;
                    }
                    i += 2;
                    continue;
                }
                if s[i] == '"' {
                    i += 1;
                    break;
                }
                m[i] = true;
                i += 1;
            }
            continue;
        }
        // 字符字面量（排除生命周期 `'a`）：`'x'` 或 `'\n'` 这种，中间那格才是"字面量内容"
        if c == '\'' && i + 2 < s.len() && s[i + 2] == '\'' {
            m[i + 1] = true;
            i += 3;
            continue;
        }
        if c == '\'' && i + 3 < s.len() && s[i + 1] == '\\' && s[i + 3] == '\'' {
            m[i + 1] = true;
            m[i + 2] = true;
            i += 4;
            continue;
        }
        i += 1;
    }
    m
}

fn ident(s: &[char], mut i: usize) -> (String, usize) {
    let mut out = String::new();
    while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == '_') {
        out.push(s[i]);
        i += 1;
    }
    (out, i)
}

fn skip_blanks(s: &[char], m: &[bool], mut i: usize) -> usize {
    while i < s.len() && (s[i].is_whitespace() || m[i]) {
        i += 1;
    }
    i
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 读一整条路径 `codes::OVERFLOW`（`ident` 在 `::` 处会停，所以得逐段接上）。
/// 只接第一段的话，命名常量永远查不到表 —— 那些点会被当成"没有编号"整批漏掉。
fn path(s: &[char], m: &[bool], mut i: usize) -> (String, usize) {
    let (mut out, mut j) = ident(s, i);
    loop {
        let k = skip_blanks(s, m, j);
        if k + 1 < s.len() && !m[k] && s[k] == ':' && s[k + 1] == ':' {
            let (seg, after) = ident(s, skip_blanks(s, m, k + 2));
            if seg.is_empty() {
                break;
            }
            out.push_str("::");
            out.push_str(&seg);
            j = after;
            continue;
        }
        break;
    }
    i = j;
    (out, i)
}

/// 从 `pos` 处的 `"` 起读字符串字面量的内容（不处理原始字符串：标题没有那种写法）。
fn next_string(s: &[char], pos: usize) -> Option<String> {
    let mut i = pos;
    while i < s.len() && s[i] != '"' {
        i += 1;
    }
    if i >= s.len() {
        return None;
    }
    i += 1;
    let mut out = String::new();
    while i < s.len() {
        match s[i] {
            '"' => return Some(out),
            '\\' => {
                i += 1;
                if i < s.len() {
                    out.push(s[i]);
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    None
}

fn trim_range(s: &[char], m: &[bool], a: usize, b: usize) -> (usize, usize) {
    let mut a = a;
    let mut b = b;
    while a < b && (s[a].is_whitespace() || m[a]) {
        a += 1;
    }
    while b > a && (s[b - 1].is_whitespace() || m[b - 1]) {
        b -= 1;
    }
    (a, b)
}

fn range_text(s: &[char], r: &(usize, usize)) -> String {
    s[r.0..r.1].iter().collect()
}

/// 把 `(`（位置 open）之后的顶层实参切成区间。已掩掉的字符不参与配对与切分，
/// 所以字符串里的逗号、`format!("{x}")` 的花括号都不会骗过它。
fn split_args(s: &[char], m: &[bool], open: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    if open >= s.len() || s[open] != '(' {
        return None;
    }
    let mut depth = 0i32;
    let mut args = Vec::new();
    let mut start = open + 1;
    let mut i = open + 1;
    while i < s.len() {
        if m[i] {
            i += 1;
            continue;
        }
        match s[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 && s[i] == ')' {
                    let r = trim_range(s, m, start, i);
                    if r.0 < r.1 {
                        args.push(r);
                    }
                    return Some((args, i));
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                let r = trim_range(s, m, start, i);
                if r.0 < r.1 {
                    args.push(r);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 本 crate 生产代码里的 `const NAME: u16 = N;`，用来还原 `codes::OVERFLOW` 这类写法。
fn const_table(src: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    let s: Vec<char> = src.chars().collect();
    let m = mask(src);
    let mut i = 0;
    while i + 5 <= s.len() {
        if m[i] || !(s[i] == 'c' && s[i..].starts_with(&['c', 'o', 'n', 's', 't'])) {
            i += 1;
            continue;
        }
        let (_, j) = ident(&s, i);
        let j = skip_blanks(&s, &m, j);
        let (name, j) = ident(&s, j);
        let mut j = skip_blanks(&s, &m, j);
        if j < s.len() && s[j] == ':' {
            j = ident(&s, skip_blanks(&s, &m, j + 1)).1;
            j = skip_blanks(&s, &m, j);
        }
        if j < s.len() && s[j] == '=' {
            let mut k = skip_blanks(&s, &m, j + 1);
            let mut num = String::new();
            while k < s.len() && !m[k] && s[k].is_ascii_digit() {
                num.push(s[k]);
                k += 1;
            }
            if !num.is_empty() && !name.is_empty() {
                if let Ok(v) = num.parse::<u16>() {
                    out.insert(name, v);
                }
            }
            j = k;
        }
        i = j.max(i + 5);
    }
    out
}

/// 找出本文件里的转发 helper（见模块注释第 2 条）。
fn trampolines(prod: &str) -> BTreeMap<String, Trampoline> {
    let s: Vec<char> = prod.chars().collect();
    let m = mask(prod);
    let mut out: BTreeMap<String, Trampoline> = BTreeMap::new();
    let mut i = 0;
    while i + 2 < s.len() {
        // 关键词 `fn`（前后都不是标识符字符）
        let hit = !m[i]
            && s[i] == 'f'
            && s[i + 1] == 'n'
            && (i == 0 || !is_ident_char(s[i - 1]))
            && (i + 2 >= s.len() || !is_ident_char(s[i + 2]));
        if !hit {
            i += 1;
            continue;
        }
        let after_fn = ident(&s, i + 2).1; // 跳过 "fn"
        let name_start = skip_blanks(&s, &m, after_fn);
        let (name, after_name) = ident(&s, name_start);
        if name.is_empty() {
            i = name_start.max(i + 2);
            continue;
        }
        let open = skip_blanks(&s, &m, after_name);
        let Some((params, close)) = split_args(&s, &m, open) else {
            i = after_name;
            continue;
        };
        // 编号参数：有且只有一个 `xxx: u16`；标题参数：第一个 `xxx: &str`。
        // 结尾必须是 ": u16"，否则 `pos: u64`、`BTreeMap<String, u16>` 这类会被误认。
        let codes: Vec<usize> = params
            .iter()
            .enumerate()
            .filter(|(_, r)| range_text(&s, r).trim_end().ends_with(": u16"))
            .map(|(k, _)| k)
            .collect();
        let titles: Vec<usize> = params
            .iter()
            .enumerate()
            .filter(|(_, r)| range_text(&s, r).trim_end().ends_with(": &str"))
            .map(|(k, _)| k)
            .collect();
        let brace = skip_blanks(&s, &m, close);
        let body_end = if brace < s.len() && s[brace] == '{' {
            match split_body(&s, &m, brace) {
                Some(end) => end,
                None => s.len(),
            }
        } else {
            s.len()
        };
        // domain 由调用方传进来的（`StkError::simple(domain: Domain, code: u16, …)`）不是转发器：
        // 它的编号本来就写在直接点上。
        let takes_domain = params
            .iter()
            .any(|r| range_text(&s, r).trim_end().ends_with(": Domain"));
        if codes.len() == 1 && !titles.is_empty() && body_end > brace && !takes_domain {
            let code_range = params[codes[0]];
            let text = range_text(&s, &code_range);
            let param_name = text.split(':').next().unwrap_or("").trim().to_string();
            out.insert(
                name,
                Trampoline {
                    code_param: param_name,
                    code_index: codes[0],
                    title_index: titles[0],
                    body: (brace + 1, body_end),
                    domain: None,
                },
            );
        }
        i = after_name.max(i + 2);
    }
    out
}

/// `{`（位置 open）到与之配对的 `}`（不含）。
fn split_body(s: &[char], m: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < s.len() {
        if m[i] {
            i += 1;
            continue;
        }
        match s[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 给每个转发 helper 定出 domain：自己的语句块里写了 `Domain::X, code`，
/// 或者转发给另一个已经认出来的 helper（`io_err` -> `err` 就是这种）。
fn resolve_domains(prod: &str, trams: &mut BTreeMap<String, Trampoline>) {
    let s: Vec<char> = prod.chars().collect();
    let m = mask(prod);
    for _ in 0..4 {
        let mut changed = false;
        let names: Vec<String> = trams.keys().cloned().collect();
        for name in names.clone() {
            let (body_a, body_b) = match trams.get(&name) {
                Some(t) if t.domain.is_none() => t.body,
                _ => continue,
            };
            let code_param = trams[&name].code_param.clone();
            // 1) 语句块里的 `Domain::X, <code 形参>`
            let mut found: Option<String> = None;
            let mut i = body_a;
            while i + 6 < body_b.min(s.len()) {
                if m[i]
                    || !(s[i] == 'D'
                        && s[i..].starts_with(&['D', 'o', 'm', 'a', 'i', 'n', ':', ':']))
                {
                    i += 1;
                    continue;
                }
                let j = skip_blanks(&s, &m, i + 6);
                if j + 1 >= s.len() || s[j] != ':' || s[j + 1] != ':' {
                    i += 1;
                    continue;
                }
                let (domain, k) = ident(&s, skip_blanks(&s, &m, j + 2));
                let k = skip_blanks(&s, &m, k);
                if k < body_b && s[k] == ',' {
                    let (arg, _) = path(&s, &m, skip_blanks(&s, &m, k + 1));
                    let last = arg.rsplit(':').next().unwrap_or("");
                    if !domain.is_empty() && last == code_param {
                        if found.is_none() {
                            found = Some(domain);
                        } else if found.as_deref() != Some(domain.as_str()) {
                            panic!("一个 helper 里把编号转发给两个不同 domain：{name}");
                        }
                    }
                }
                i = k.max(i + 1);
            }
            if found.is_none() {
                // 2) 转发给另一个已知 helper：`err(code, title)`
                for other in names.iter() {
                    if other == &name {
                        continue;
                    }
                    let Some(t2) = trams.get(other) else { continue };
                    if t2.domain.is_none() {
                        continue;
                    }
                    let mut p = body_a;
                    while let Some(rel) = find_ident(&s, &m, p, other) {
                        if rel.1 >= body_b {
                            break;
                        }
                        let open = skip_blanks(&s, &m, rel.0 + other.len());
                        let Some((args, _)) = split_args(&s, &m, open) else {
                            break;
                        };
                        if args.get(t2.code_index).map(|r| range_text(&s, r))
                            == Some(code_param.clone())
                        {
                            found = t2.domain.clone();
                            break;
                        }
                        p = open + 1;
                    }
                    if found.is_some() {
                        break;
                    }
                }
            }
            if let Some(d) = found {
                trams.get_mut(&name).unwrap().domain = Some(d);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// 这个标识符是不是紧跟在 `fn` 关键词后面的函数名（= 定义，不是调用点）。
fn preceded_by_fn(s: &[char], start: usize) -> bool {
    let mut i = start;
    while i > 0 && (s[i - 1].is_whitespace() || s[i - 1] == '#') {
        i -= 1;
    }
    i >= 2 && s[i - 2] == 'f' && s[i - 1] == 'n' && (i == 2 || !is_ident_char(s[i - 3]))
}

/// 从 `from` 起找下一个作为独立标识符出现的 `want`，返回 (起始, 起始+长度)。
fn find_ident(s: &[char], m: &[bool], from: usize, want: &str) -> Option<(usize, usize)> {
    let w: Vec<char> = want.chars().collect();
    let mut i = from;
    while i + w.len() <= s.len() {
        if m[i] || s[i] != w[0] {
            i += 1;
            continue;
        }
        if !s[i..].starts_with(&w) {
            i += 1;
            continue;
        }
        if i > 0 && (is_ident_char(s[i - 1]) || s[i - 1] == ':') {
            i += 1;
            continue;
        }
        let after = i + w.len();
        if after < s.len() && is_ident_char(s[after]) {
            i += 1;
            continue;
        }
        return Some((i, after));
    }
    None
}

/// 扫直接点 `Domain::X, N, "标题"`；`trams` 用来认出"这是 helper 的定义体，不是调用点"。
fn scan_direct(
    prod: &str,
    krate: &str,
    file: &str,
    consts: &BTreeMap<String, u16>,
    trams: &BTreeMap<String, Trampoline>,
    seen: &mut Seen,
    unresolved: &mut Vec<String>,
) {
    let s: Vec<char> = prod.chars().collect();
    let m = mask(prod);
    let needle = ['D', 'o', 'm', 'a', 'i', 'n'];
    let mut i = 0;
    while i + needle.len() <= s.len() {
        if m[i] || !(s[i] == 'D' && s[i..].starts_with(&needle)) {
            i += 1;
            continue;
        }
        let j = skip_blanks(&s, &m, i + needle.len());
        if j + 1 >= s.len() || s[j] != ':' || s[j + 1] != ':' {
            i = j.max(i + 1);
            continue;
        }
        let (domain, k) = ident(&s, skip_blanks(&s, &m, j + 2));
        let k = skip_blanks(&s, &m, k);
        if k >= s.len() || s[k] != ',' {
            i = k.max(i + 1);
            continue;
        }
        let (arg, after) = path(&s, &m, skip_blanks(&s, &m, k + 1));
        let short = arg.rsplit(':').next().unwrap_or("").to_string();
        // helper 定义体里的 `Domain::X, code` 由调用点那一路负责，这里不算漏
        if trams
            .values()
            .any(|t| t.code_param == short && i > t.body.0 && i < t.body.1)
        {
            i = after;
            continue;
        }
        let Some(code) = short
            .parse::<u16>()
            .ok()
            .or_else(|| consts.get(&short).copied())
        else {
            unresolved.push(format!("{file}: Domain::{domain} 的编号是变量 {arg}"));
            i = after;
            continue;
        };
        let Some(title) = literal_arg(&s, &m, after) else {
            unresolved.push(format!("{file}: Domain::{domain}/{code} 的标题不是字面量"));
            i = after;
            continue;
        };
        record(seen, krate, file, &domain, code, title);
        i = after;
    }
}

/// 逗号之后的字面量标题（允许 `Some("…")`、`&format!` 直接被拒 —— 猜不得）。
fn literal_arg(s: &[char], m: &[bool], mut i: usize) -> Option<String> {
    i = skip_blanks(s, m, i);
    if i >= s.len() || s[i] != ',' {
        return None;
    }
    i = skip_blanks(s, m, i + 1);
    while i < s.len() && !m[i] && (is_ident_char(s[i]) || s[i] == ':') {
        let (_, j) = ident(s, i);
        i = skip_blanks(s, m, j);
        if i < s.len() && (s[i] == '(' || s[i] == '!') {
            i = skip_blanks(s, m, i + 1);
            continue;
        }
        return None;
    }
    if i < s.len() && !m[i] && s[i] == '"' {
        return next_string(s, i);
    }
    None
}

/// 扫转发调用点：`cli_err(150, "无法读取 batch spec")`。
fn scan_trampoline_calls(
    prod: &str,
    krate: &str,
    file: &str,
    trams: &BTreeMap<String, Trampoline>,
    seen: &mut Seen,
    unresolved: &mut Vec<String>,
) {
    let s: Vec<char> = prod.chars().collect();
    let m = mask(prod);
    for (name, t) in trams {
        let Some(domain) = t.domain.clone() else {
            continue;
        };
        let mut from = 0;
        while let Some((start, after)) = find_ident(&s, &m, from, name) {
            from = after;
            // `fn err(code: u16…)` 的定义本身不是调用点
            if preceded_by_fn(&s, start) {
                continue;
            }
            let open = skip_blanks(&s, &m, after);
            if open >= s.len() || s[open] != '(' {
                continue;
            }
            let Some((args, close)) = split_args(&s, &m, open) else {
                from = open + 1;
                continue;
            };
            let inside_body = start > t.body.0 && start < t.body.1;
            let Some(arg) = args.get(t.code_index) else {
                from = open + 1;
                continue;
            };
            let text: String = s[arg.0..arg.1]
                .iter()
                .enumerate()
                .filter(|(k, _)| !m[arg.0 + k])
                .map(|(_, c)| *c)
                .collect();
            if text.chars().all(|c| c.is_ascii_digit()) && !text.is_empty() {
                let Ok(code) = text.parse::<u16>() else {
                    from = close + 1;
                    continue;
                };
                let title = args
                    .get(t.title_index)
                    .and_then(|r| {
                        let mut a = r.0;
                        while a < r.1 && (s[a].is_whitespace() || s[a] == '&' || m[a]) {
                            a += 1;
                        }
                        if a < r.1 && s[a] == '"' {
                            next_string(&s, a)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "<标题不是字面量>".to_string());
                if title == "<标题不是字面量>" {
                    unresolved.push(format!("{file}: {name}({code}, …) 的标题不是字面量"));
                } else {
                    record(seen, krate, file, &domain, code, title);
                }
            } else if !inside_body {
                unresolved.push(format!("{file}: {name}({text}, …) 的编号是变量"));
            }
            from = close + 1;
        }
    }
}

fn record(seen: &mut Seen, krate: &str, file: &str, domain: &str, code: u16, title: String) {
    seen.entry((domain.to_string(), code))
        .or_default()
        .entry((krate.to_string(), title))
        .or_default()
        .push(file.to_string());
}

#[test]
fn no_two_crates_share_an_error_code_with_different_meanings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/core-model 的上两级是仓库根")
        .join("crates");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(files.len() > 40, "扫到的源文件太少：{}", files.len());

    // 先按 crate 收齐生产代码
    let mut prod_by_file: BTreeMap<String, (String, String)> = BTreeMap::new();
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(f.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if !rel.contains("/src/") {
            continue; // tests/ 与 examples/ 里的编号是测试样例，不是对外契约
        }
        let krate = rel.split('/').next().unwrap_or("?").to_string();
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        prod_by_file.insert(rel, (krate, production_only(&src).to_string()));
    }
    assert!(
        prod_by_file.len() > 30,
        "生产源码文件太少：{}",
        prod_by_file.len()
    );

    // 命名常量按 crate 汇总：常量和使用点常常不在同一个文件，按单文件解析会整批漏掉。
    let mut crate_consts: BTreeMap<String, BTreeMap<String, u16>> = BTreeMap::new();
    let mut per_file: BTreeMap<String, BTreeMap<String, Trampoline>> = BTreeMap::new();
    for (rel, (krate, prod)) in &prod_by_file {
        let table = crate_consts.entry(krate.clone()).or_default();
        for (name, value) in const_table(prod) {
            if let Some(prev) = table.get(&name) {
                assert_ne!(
                    *prev, value,
                    "crate {krate} 里常量 {name} 同时等于 {prev} 和 {value}，编号无法确定"
                );
            }
            table.insert(name, value);
        }
        let mut trams = trampolines(prod);
        resolve_domains(prod, &mut trams);
        per_file.insert(rel.clone(), trams);
    }
    let helpers: Vec<usize> = per_file
        .values()
        .map(|t| t.values().filter(|tr| tr.domain.is_some()).count())
        .collect();
    let helper_count: usize = helpers.iter().sum();
    let unknown_helpers: Vec<String> = per_file
        .iter()
        .flat_map(|(f, t)| {
            t.iter()
                .filter(|(_, tr)| tr.domain.is_none())
                .map(move |(n, _)| format!("{f}: {n}"))
        })
        .collect();
    assert!(
        unknown_helpers.is_empty(),
        "有 helper 认不出 domain，编号就没进对照表：{}\n\
         （认不出不等于没有 —— 补上识别规则，别让它假绿）",
        unknown_helpers.join("\n")
    );

    let mut seen: Seen = BTreeMap::new();
    let mut unresolved: Vec<String> = Vec::new();
    for (rel, (krate, prod)) in &prod_by_file {
        let consts = crate_consts.get(krate).cloned().unwrap_or_default();
        let trams = &per_file[rel];
        scan_direct(prod, krate, rel, &consts, trams, &mut seen, &mut unresolved);
        scan_trampoline_calls(prod, krate, rel, trams, &mut seen, &mut unresolved);
    }
    let skipped = unresolved.len();
    assert!(!seen.is_empty(), "一个错误码都没扫到：解析器坏了，不是通过");
    assert!(
        seen.len() > 190,
        "只扫到 {} 个 (domain,code)：覆盖面退化了，先证明解析器还认得全部写法",
        seen.len()
    );

    if std::env::var("STK_DUMP_CODES").is_ok() {
        // 换编号时需要"这个域里哪些数已经用了"的权威答案，
        // 而那份答案只有这个扫描器能给 —— 另写一个脚本就会和这里走偏。
        let mut by_domain: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for ((domain, code), by_use) in &seen {
            let mut uses = Vec::new();
            for (k, t) in by_use.keys() {
                uses.push(format!("{k}:{t}"));
            }
            by_domain
                .entry(domain.as_str())
                .or_default()
                .push(format!("{code} => {}", uses.join(" | ")));
        }
        for (domain, rows) in by_domain {
            eprintln!(
                "[{domain}] {} 个编号\n    {}",
                rows.len(),
                rows.join("\n    ")
            );
        }
    }

    let (bad, families, shared) = classify(&seen);
    eprintln!(
        "{} 个 (domain,code)、{} 处使用、{} 个转发 helper；{shared} 个编号是跨 crate 刻意同义复用（标题一致）、\
         {families} 个编号在单个 crate 内承载一族错误（既有风格，见 H8）；{skipped} 处编号或标题不是字面量（列在下面）",
        seen.len(),
        seen.values().map(|v| v.values().map(|w| w.len()).sum::<usize>()).sum::<usize>(),
        helper_count,
    );
    if !unresolved.is_empty() {
        eprintln!(
            "没能归入对照表的点：\n{}",
            unresolved
                .iter()
                .map(|u| format!("    {u}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(
        bad.is_empty(),
        "这些编号被两个 crate 各自用来表达不同的事（换编号，或让标题逐字一致）：\n{}\n\
         如果是有意的共享，请统一标题文字，别靠 details 区分跨 crate 的不同含义。",
        bad.join("\n")
    );
}

/// 撞码判据。抽成函数是为了能在下面用**故意写坏的样本**验证这个把关器本身 ——
/// 解析器坏掉时的表现是"零撞码=通过"，只跑真树永远看不出来。
fn classify(seen: &Seen) -> (Vec<String>, usize, usize) {
    let mut bad = Vec::new();
    let mut families = 0;
    let mut shared = 0;
    for ((domain, code), by_use) in seen {
        let mut crates: BTreeSet<&String> = BTreeSet::new();
        let mut titles: BTreeSet<&String> = BTreeSet::new();
        for (k, t) in by_use.keys() {
            crates.insert(k);
            titles.insert(t);
        }
        // 判据是"标题有几种说法"，不是"出现了几次"：两个 crate 用同一个编号说同一件事
        // （标题逐字相同）是本仓库刻意允许的共享（transform-time 复用 core-model 的 PTS 码）。
        if titles.len() < 2 {
            if crates.len() > 1 {
                shared += 1;
            }
            continue;
        }
        if crates.len() > 1 {
            let detail = by_use
                .iter()
                .map(|((k, t), w)| format!("      {k} {t:?} <- {}", w.join(", ")))
                .collect::<Vec<_>>()
                .join("\n");
            bad.push(format!("    {domain:?}/{code}:\n{detail}"));
        } else {
            families += 1;
        }
    }
    (bad, families, shared)
}

#[test]
fn the_checker_still_detects_a_deliberate_collision() {
    // 把关器的负基线：真树绿了不代表它在查东西。
    let consts = BTreeMap::new();
    let trams = BTreeMap::new();
    let mut seen: Seen = BTreeMap::new();
    let mut unresolved = Vec::new();
    let a = "fn x() { StkError::simple(Domain::Pgs, 7, \"甲说的事\"); }";
    let b = "fn y() { StkError::simple(Domain::Pgs, codes::SEVEN, \"乙说的事\"); }";
    scan_direct(
        a,
        "crate-a",
        "a.rs",
        &consts,
        &trams,
        &mut seen,
        &mut unresolved,
    );
    let mut with_const = BTreeMap::new();
    with_const.insert("SEVEN".to_string(), 7u16);
    scan_direct(
        b,
        "crate-b",
        "b.rs",
        &with_const,
        &trams,
        &mut seen,
        &mut unresolved,
    );
    let (bad, families, shared) = classify(&seen);
    assert_eq!(bad.len(), 1, "故意撞码没被查出来：{bad:?}");
    assert!(bad[0].contains("\"Pgs\"/7"), "{}", bad[0]);
    assert_eq!((families, shared), (0, 0), "{families} {shared}");
    assert!(unresolved.is_empty(), "{unresolved:?}");

    // 同一编号、两个 crate、标题逐字相同 = 刻意的同义复用，不该算撞码
    let mut seen2: Seen = BTreeMap::new();
    scan_direct(
        "fn x() { StkError::simple(Domain::Pgs, 7, \"甲说的事\"); }",
        "crate-a",
        "a.rs",
        &consts,
        &trams,
        &mut seen2,
        &mut unresolved,
    );
    scan_direct(
        "fn z() { StkError::simple(Domain::Pgs, 7, \"甲说的事\"); }",
        "crate-c",
        "c.rs",
        &consts,
        &trams,
        &mut seen2,
        &mut unresolved,
    );
    let (bad2, _f, shared2) = classify(&seen2);
    assert!(bad2.is_empty(), "标题一致却被当成撞码：{bad2:?}");
    assert_eq!(shared2, 1, "同义复用应当被数出来");
}

#[test]
fn comments_and_string_contents_are_not_error_codes() {
    // 掩码是把关的一部分：文档里举的例子不能变成"真存在的一个码"，
    // 否则一次注释改动就能凭空造出（或掩盖）一个撞码。
    let consts = BTreeMap::new();
    let trams = BTreeMap::new();
    let mut seen: Seen = BTreeMap::new();
    let mut unresolved = Vec::new();
    let src = "//! 例如 `Domain::Pgs, 7, \"注释里的例子\"`\nfn ok() {\n    let m = format!(\"Domain::Pgs, 8, \\\"字符串里的\\\"\");\n    let _ = m;\n}\n";
    scan_direct(
        src,
        "crate-a",
        "a.rs",
        &consts,
        &trams,
        &mut seen,
        &mut unresolved,
    );
    assert!(
        seen.is_empty(),
        "注释/字符串里的写法被当成了错误码：{seen:?}"
    );
}

#[test]
fn a_trampoline_helper_feeds_its_call_sites_into_the_table() {
    // 转发点是覆盖面里最容易整批漏掉的一格（CLI 的 110+ 全走这条路）。
    let consts = BTreeMap::new();
    let prod = "fn cli_err(code: u16, msg: &str) -> StkError {\n    StkError::simple(Domain::Input, code, msg)\n}\n\
                fn use_it() -> StkError {\n    cli_err(150, \"读不到\")\n}\n\
                fn chain(e: std::io::Error, code: u16, title: &str) -> StkError {\n    cli_err(code, title).with_detail(e.to_string())\n}\n";
    let mut trams = trampolines(prod);
    resolve_domains(prod, &mut trams);
    let mut seen: Seen = BTreeMap::new();
    let mut unresolved = Vec::new();
    scan_direct(
        prod,
        "cli",
        "x.rs",
        &consts,
        &trams,
        &mut seen,
        &mut unresolved,
    );
    scan_trampoline_calls(prod, "cli", "x.rs", &trams, &mut seen, &mut unresolved);
    assert!(
        trams.contains_key("cli_err"),
        "{:?}",
        trams.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        trams["cli_err"].domain.as_deref(),
        Some("Input"),
        "helper 的 domain 没认出来"
    );
    assert_eq!(
        trams["chain"].domain.as_deref(),
        Some("Input"),
        "二级转发（io_err -> err）没继承到 domain，那一族的编号会整批漏掉"
    );
    // helper 自己的 `Domain::Input, code` 不该被算成"没解析出编号"
    assert!(
        !unresolved
            .iter()
            .any(|u| u.contains("Domain::Input 的编号是变量")),
        "{unresolved:?}"
    );
    let keys: Vec<(String, u16)> = seen.keys().cloned().collect();
    assert!(keys.contains(&("Input".to_string(), 150u16)), "{keys:?}");
}
