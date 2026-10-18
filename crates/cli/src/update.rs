//! 28 §6 的**更新检查**（第一版）。
//!
//! ## 规格怎么说的
//!
//! > 第一版可只"检查更新并打开 release 页面"；成熟后做 signed manifest + staged
//! > download。**不要在早期自写高权限 auto-updater。**
//!
//! 本模块严格落在第一版范围内。它：
//!
//! 1. **不下载、不安装、不提权**。只回答"有没有新版本"并给出一个 URL。
//! 2. **不做网络请求**。当前的版本号从配置文件或命令行读入；真正的 HTTP 获取
//!    由调用方（GUI 或脚本）用平台已信任的通道做。理由见下。
//! 3. 可以**只离线工作**：给一个版本清单文件就能比较。
//!
//! ## 为什么不引 HTTP 客户端
//!
//! 三条理由，按重要性排序：
//!
//! **一，它会把一个"读版本号"的功能变成一条会走网络的代码路径。** 本项目至今
//! 没有任何出网依赖（`cargo tree` 可验），而一个更新检查是**唯一**会主动出网的
//! 功能。把它做成一条独立、可审计、可整体关闭的路径，比把 `reqwest` 拖进依赖树
//! （连带 TLS 栈、根证书策略、代理语义）要小得多。
//!
//! **二，21 §1 的许可政策**。TLS 栈那一族依赖的许可需要逐项过审，而这条功能
//! 的收益（少点一次浏览器）远不抵那份长期维护成本。
//!
//! **三，安全边界**。更新检查是供应链攻击的经典入口。一个**不下载任何东西**的
//! 检查最多泄露"有人在用这个版本"，而一个会下载并解压的实现会把攻击面从
//! "解析一个 JSON" 扩大到"解析网络响应 + 落盘 + （若做自动更新）执行"。
//! 规格明确说"不要在早期自写高权限 auto-updater"——本模块干脆不做下载侧。
//!
//! ## 版本比较为什么不用第三方 semver crate
//!
//! 只需要比 `MAJOR.MINOR.PATCH`（`Cargo.toml` 里就是这个形状），
//! 而 semver 的完整语法（预发布、构建元数据、区间）在这里全是负担。
//! 更关键的是：**比较规则的每一条都要能被测试钉住**，而自己写的三十行
//! 比"依赖一个 crate 的某个版本的语义"更容易核对。
//! 遇到本函数不认识的形状时**返回 `Unknown` 而不是猜** —— 这是本模块最重要的一条。

use core_model::error::{Domain, Result, StkError};

/// 一次版本比较的结论。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// 远端更新。
    Newer,
    /// 同一个版本。
    Same,
    /// 本地更新（用户装的是更新的版本，或清单是旧的）。
    Older,
    /// 比对不了。**不猜**，并说明为什么。
    Unknown,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Newer => "有新版本",
            Self::Same => "已是最新",
            Self::Older => "本地版本更新",
            Self::Unknown => "无法判断",
        }
    }
}

/// 解析 `MAJOR.MINOR.PATCH`。
///
/// 只接受三段的纯数字。少一段、多一段、带 `-`/`+`、带前导 `v`，一律 `None`。
///
/// ## 为什么这么严
///
/// 宽松解析（"`1.2` 就当成 `1.2.0`"、"`v1.2.3` 就去掉 v"）看着友好，
/// 但它会把**清单文件的格式错误**变成一次看似成功的比较。
/// 更新检查的两种失败里：
///
/// - 假阴性（明明有新版本却说没有）→ 用户留在旧版本上，还能用；
/// - 假阳性（说 `1.2.3` 比 `1.2.0` 新，其实读错了字段）→ 用户去下载一个错的东西。
///
/// 后者更坏。所以宁可 `Unknown` 并说清楚是哪个字符串解析不了。
fn parse_triple(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.');
    let a = it.next()?.parse::<u64>().ok()?;
    let b = it.next()?.parse::<u64>().ok()?;
    let c = it.next()?.parse::<u64>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

/// 比较两个版本号。
///
/// **任一侧解析不了就 `Unknown`**，而不是当成 0 或"相等"。
pub fn compare(local: &str, remote: &str) -> (Verdict, String) {
    let (Some(l), Some(r)) = (parse_triple(local), parse_triple(remote)) else {
        let which = if parse_triple(local).is_none() {
            format!("本地版本「{local}」")
        } else {
            format!("清单里的版本「{remote}」")
        };
        return (
            Verdict::Unknown,
            format!("{which}不是 MAJOR.MINOR.PATCH 形状，无法比对（不猜）"),
        );
    };
    let v = if r > l {
        Verdict::Newer
    } else if r == l {
        Verdict::Same
    } else {
        Verdict::Older
    };
    (
        v,
        format!("本地 {}.{}.{}，清单 {}.{}.{}", l.0, l.1, l.2, r.0, r.1, r.2),
    )
}

/// 更新通道的投影信息：给用户看的全部内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub local: String,
    pub remote: String,
    pub verdict: Verdict,
    pub detail: String,
    /// 打开这个地址就能拿到产物与说明。**本程序不会自己去访问它**。
    pub release_url: String,
}

/// 默认的发布页地址。
///
/// 这个值在 `--release-url` 或清单里能被覆盖，因为本仓库当前没有公开的
/// release 站点（28 §1 的发布链仍未走完）。**不编一个看起来像真的地址** ——
/// 用户照着点会 404。默认值指向仓库本身，那至少是一个真实存在的地方。
pub const DEFAULT_RELEASE_URL: &str = "https://github.com/example/subtitletoolkit/releases";

/// 从一份清单 JSON 里取出版本与 URL。
///
/// 清单的形状刻意做成最小的：
///
/// ```json
/// { "version": "0.2.0", "url": "https://.../releases/tag/v0.2.0" }
/// ```
///
/// 不引 `serde_json` 之外的东西 —— 它已在依赖树里（CLI 一直在用）。
pub fn parse_manifest(text: &str) -> Result<(String, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        StkError::simple(Domain::Input, 210, "版本清单不是合法 JSON").with_detail(e.to_string())
    })?;
    let version = v.get("version").and_then(|x| x.as_str()).ok_or_else(|| {
        StkError::simple(
            Domain::Input,
            211,
            "版本清单缺少 version 字段（必须是字符串）",
        )
    })?;
    let url = v.get("url").and_then(|x| x.as_str()).map(str::to_string);
    Ok((version.to_string(), url))
}

/// 组装一份报告。`manifest` 是清单文本；`None` 表示用户没给清单。
///
/// 没给清单时返回的是 `Unknown` + 一个"去哪儿看"的指引 —— **不假装检查过了**。
/// 这一点很要紧：一个静默返回"已是最新"的更新检查，比没有更新检查更坏，
/// 因为它会让用户确信自己是最新的。
pub fn report(local: &str, manifest: Option<&str>, release_url: Option<&str>) -> Result<Report> {
    let url = release_url.unwrap_or(DEFAULT_RELEASE_URL).to_string();
    let Some(text) = manifest else {
        return Ok(Report {
            local: local.to_string(),
            remote: String::new(),
            verdict: Verdict::Unknown,
            detail: format!(
                "没有给版本清单（--manifest FILE），无法判断是否有新版本。\
                 打开 {url} 可以看有哪些版本。"
            ),
            release_url: url,
        });
    };
    let (remote, from_manifest) = parse_manifest(text)?;
    let (verdict, detail) = compare(local, &remote);
    Ok(Report {
        local: local.to_string(),
        remote,
        verdict,
        detail,
        release_url: from_manifest.unwrap_or(url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_remote_is_reported_as_newer() {
        assert_eq!(compare("0.1.0", "0.2.0").0, Verdict::Newer);
        assert_eq!(compare("0.1.0", "1.0.0").0, Verdict::Newer);
        assert_eq!(compare("1.2.3", "1.2.4").0, Verdict::Newer);
    }

    #[test]
    fn equal_is_same() {
        assert_eq!(compare("0.1.0", "0.1.0").0, Verdict::Same);
    }

    #[test]
    fn older_remote_is_older_not_an_error() {
        // 用户装的是更新的版本（或清单是旧的）。这不是错误。
        assert_eq!(compare("0.2.0", "0.1.0").0, Verdict::Older);
    }

    #[test]
    fn numeric_ordering_is_not_string_ordering() {
        // 字符串比较会把 "10.0.0" 判成小于 "9.0.0"。这条钉住它不是字符串比较。
        assert_eq!(compare("9.0.0", "10.0.0").0, Verdict::Newer);
    }

    #[test]
    fn unparsable_versions_are_unknown_and_say_which() {
        for bad in ["1.2", "1.2.3.4", "v1.2.3", "1.2.x", "", "1.2.3-beta"] {
            let (v, why) = compare("0.1.0", bad);
            assert_eq!(v, Verdict::Unknown, "{bad} 不该被解析成功");
            assert!(why.contains("无法比对"), "{bad} 的说明没说清: {why}");
        }
        // 本地侧坏也一样
        let (v, why) = compare("v0.1", "0.1.0");
        assert_eq!(v, Verdict::Unknown);
        assert!(why.contains("本地版本"), "说明没说清是哪一侧: {why}");
    }

    #[test]
    fn unknown_is_never_silently_treated_as_same() {
        // 最重要的一条：解析失败**不能**退化成“已是最新”。
        // 那会让用户确信自己是最新的，而他其实不知道。
        let (v, _) = compare("garbage", "0.1.0");
        assert_ne!(v, Verdict::Same);
        assert_eq!(v, Verdict::Unknown);
    }

    #[test]
    fn a_manifest_is_parsed_at_its_minimum_shape() {
        let (v, u) = parse_manifest(r#"{"version":"0.2.0"}"#).unwrap();
        assert_eq!(v, "0.2.0");
        assert_eq!(u, None);
    }

    #[test]
    fn a_manifest_can_carry_its_own_url() {
        let (v, u) = parse_manifest(r#"{"version":"0.2.0","url":"https://x/y"}"#).unwrap();
        assert_eq!(v, "0.2.0");
        assert_eq!(u.as_deref(), Some("https://x/y"));
    }

    #[test]
    fn a_bad_manifest_says_what_is_wrong() {
        // 不是 JSON
        let e = parse_manifest("not json").unwrap_err();
        assert!(e.title.contains("不是合法 JSON"), "{}", e.title);
        // 是 JSON 但没有 version
        let e = parse_manifest(r#"{"ver":"1"}"#).unwrap_err();
        assert!(e.title.contains("缺少 version"), "{}", e.title);
    }

    #[test]
    fn without_a_manifest_we_do_not_pretend_to_have_checked() {
        // 这条是“不假装检查过”的回归用例。
        let r = report("0.1.0", None, None).unwrap();
        assert_eq!(r.verdict, Verdict::Unknown);
        assert!(
            r.detail.contains("无法判断"),
            "没清单时不该给出结论: {}",
            r.detail
        );
        assert!(
            r.detail.contains("--manifest"),
            "要告诉用户怎么补: {}",
            r.detail
        );
        assert!(!r.release_url.is_empty(), "至少要给一个可以去看的地方");
    }

    #[test]
    fn a_manifest_overrides_the_release_url() {
        let r = report(
            "0.1.0",
            Some(r#"{"version":"0.2.0","url":"https://a/b"}"#),
            Some("https://fallback/x"),
        )
        .unwrap();
        assert_eq!(r.verdict, Verdict::Newer);
        assert_eq!(r.release_url, "https://a/b");
    }

    #[test]
    fn the_release_url_parameter_wins_over_the_builtin_default() {
        let r = report("0.1.0", None, Some("https://mine/x")).unwrap();
        assert_eq!(r.release_url, "https://mine/x");
    }
}
