//! 把 36 §1 的 ToolDefaults 真正接到命令上。
//!
//! ## 为什么解析与求值要分开
//!
//! `args.rs` 只记"用户给了什么"（`Option<T>`），执行阶段才按
//!
//! ```text
//! 用户给的值  →  ToolDefaults 里的值  →  编译期默认值
//! ```
//!
//! 这个顺序求值。三个来源各自的地位不同：
//!
//! 1. **用户显式给的**永远优先 —— 命令行是当下的意图，设置文件是过去的偏好。
//! 2. **ToolDefaults** 是用户跨会话的偏好，但它**不覆盖**本次命令行的显式选择。
//! 3. **编译期默认值**是兜底，且它只有一份（在 [`PgsEncodeDefaults`] /
//!    [`RenderDefaults`] 这些结构里）。不在别处再写第二份 ——
//!    两份默认值定义分叉的后果是"同一件事在两个入口表现不同"。
//!
//! ## 为什么把求值收在一个模块里
//!
//! 每个命令各自去查 ToolDefaults 会有两个问题：一是"用户值 vs 默认值"的优先级
//! 会被各处重新解释一遍（迟早不一致）；二是**无法统一回答"这次跑的到底是哪些
//! 取值"** —— 而那句话（20 §Repro）是复现问题的前提。
//! 收在这里之后，"求值"变成可单独测试的纯函数。
//!
//! ## 解析失败时怎么办
//!
//! 设置里的值可能拼错（`--level standard` 写成了 `standardd`）。
//! 处理方式与命令行参数错**一样**：报错并退出，而不是静默用默认值 ——
//! 静默退回会让用户以为自己的设置生效了，而他其实在用别的值。
//! 差异只在错误码与"改法"上：设置错的改法指向 `tool-defaults set`。

use core_model::error::{Domain, Result, StkError};
use core_model::profile::ValidationLevel;
use core_model::time::Fps;
use core_model::tool_defaults::{self as td, Tool};
use core_model::Canvas;

/// `pgs encode` 的最终取值（三个来源求值之后）。
#[derive(Clone, Debug, PartialEq)]
pub struct PgsEncodeDefaults {
    pub fps: Fps,
    pub canvas: Canvas,
    pub level: ValidationLevel,
    pub default_font: String,
}

impl Default for PgsEncodeDefaults {
    fn default() -> Self {
        PgsEncodeDefaults {
            // 与 `args.rs` 的原始默认一致：23.976 / 1920x1080 / standard / Arial
            fps: Fps::NTSC_23976,
            canvas: Canvas::FHD1080,
            level: ValidationLevel::Standard,
            default_font: "Arial".to_string(),
        }
    }
}

/// `render` 的最终取值。
#[derive(Clone, Debug, PartialEq)]
pub struct RenderDefaults {
    pub fps: Fps,
    pub canvas: Canvas,
    pub default_font: String,
    pub min_axis: u32,
}

impl Default for RenderDefaults {
    fn default() -> Self {
        RenderDefaults {
            fps: Fps::NTSC_23976,
            canvas: Canvas::FHD1080,
            default_font: "Arial".to_string(),
            min_axis: 0,
        }
    }
}

/// 求值的结果：值 + 每一项**来自哪**。
///
/// "来自哪"是给报告用的：用户问"我设的生效了吗"，只有取值不回答那个问题。
#[derive(Clone, Debug, PartialEq)]
pub struct Sourced<T> {
    pub value: T,
    /// 键 → 来源标签（`命令行` / `tool-defaults` / `内置默认`）。
    pub sources: std::collections::BTreeMap<String, &'static str>,
}

/// 从 ToolDefaults 里取一个字符串键。
fn s(d: &td::ToolDefaults, tool: Tool, key: &str) -> Option<String> {
    td::resolve(d, tool).effective.get(key).cloned()
}

/// 取一个整数键。设置里的值可能是 JSON 数字，也可能是字符串 —— 两种都收，
/// 但**值不对就报错**而不是静默用默认值。
fn u32_of(d: &td::ToolDefaults, tool: Tool, key: &str) -> Result<Option<u32>> {
    let Some(raw) = s(d, tool, key) else {
        return Ok(None);
    };
    raw.parse::<u32>().map(Some).map_err(|_| {
        let tool_name = tool.name();
        StkError::simple(Domain::Input, 219, "工具默认值里这一项不是合法的非负整数")
            .with_detail(format!("{tool_name}.{key} = {raw:?}"))
            .with_action(
                format!(
                    "改成非负整数，或删掉它后用 `tool-defaults set {tool_name} {key} <值>` 重设"
                )
                .as_str(),
            )
    })
}

/// 取一个布尔键。
fn bool_of(d: &td::ToolDefaults, tool: Tool, key: &str) -> Result<Option<bool>> {
    let Some(raw) = s(d, tool, key) else {
        return Ok(None);
    };
    match raw.as_str() {
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        _ => {
            let tool_name = tool.name();
            Err(
                StkError::simple(Domain::Input, 219, "工具默认值里这一项不是布尔值")
                    .with_detail(format!("{tool_name}.{key} = {raw:?}"))
                    .with_action("改成 true 或 false"),
            )
        }
    }
}

/// 取帧率。接受 `24000/1001` 这样的有理数，也接受 `23.976` 这样的标签。
fn fps_of(d: &td::ToolDefaults, tool: Tool, key: &str) -> Result<Option<Fps>> {
    let Some(raw) = s(d, tool, key) else {
        return Ok(None);
    };
    Fps::from_label(&raw)
        .ok()
        .or_else(|| {
            raw.split_once('/').and_then(|(n, den)| {
                let n = n.trim().parse::<i64>().ok()?;
                let den = den.trim().parse::<i64>().ok()?;
                core_model::time::Rational::new(n, den)
                    .ok()
                    .and_then(|r| Fps::new(r).ok())
            })
        })
        .map(Some)
        .ok_or_else(|| {
            let tool_name = tool.name();
            StkError::simple(Domain::Input, 219, "工具默认值里的帧率无法解析")
                .with_detail(format!("{tool_name}.{key} = {raw:?}"))
                .with_action("用 `24000/1001` 或 `23.976` 这样的写法")
        })
}

/// 取画布。必须是 `WxH`。
fn canvas_of(d: &td::ToolDefaults, tool: Tool, key: &str) -> Result<Option<Canvas>> {
    let Some(raw) = s(d, tool, key) else {
        return Ok(None);
    };
    let parsed = raw.split_once(['x', 'X']).and_then(|(w, h)| {
        let w = w.trim().parse::<u32>().ok()?;
        let h = h.trim().parse::<u32>().ok()?;
        Canvas::new(w, h).ok()
    });
    // 解析不了就报错，**不静默退回默认值** —— 静默退回会让用户以为
    // 自己的设置生效了，而他其实在用别的值（与命令行参数错的取向一致）。
    match parsed {
        Some(c) => Ok(Some(c)),
        None => {
            let tool_name = tool.name();
            Err(
                StkError::simple(Domain::Input, 219, "工具默认值里的画布无法解析")
                    .with_detail(format!("{tool_name}.{key} = {raw:?}"))
                    .with_action(
                        format!(
                            "用 `1920x1080` 这样的写法，或删掉它：tool-defaults set {tool_name} {key} 1920x1080"
                        )
                        .as_str(),
                    ),
            )
        }
    }
}

fn level_of(d: &td::ToolDefaults, tool: Tool, key: &str) -> Result<Option<ValidationLevel>> {
    let Some(raw) = s(d, tool, key) else {
        return Ok(None);
    };
    crate::args::parse_level_public(&raw)
        .map(Some)
        .ok_or_else(|| {
            let tool_name = tool.name();
            StkError::simple(Domain::Input, 219, "工具默认值里的校验级别无法解析")
                .with_detail(format!("{tool_name}.{key} = {raw:?}"))
                // 改法要能**照着做**。只说“只能是 probe|...|strict”不够 ——
                // 用户已经知道取值集；他需要知道改的是哪一项、用什么命令。
                .with_action(
                    format!(
                        "只能是 probe|structure|basic|standard|strict；改它就跑：tool-defaults set \"{tool_name}\" {key} standard"
                    )
                    .as_str(),
                )
        })
}

/// 求 `pgs encode` 的最终取值。
///
/// `cli_*` 是命令行给的值（`None` = 用户没给）。
pub fn pgs_encode(
    d: &td::ToolDefaults,
    cli_fps: Option<Fps>,
    cli_canvas: Option<Canvas>,
    cli_level: Option<ValidationLevel>,
    cli_font: Option<&str>,
) -> Result<Sourced<PgsEncodeDefaults>> {
    let base = PgsEncodeDefaults::default();
    let t = Tool::PgsEncode;
    let mut sources = std::collections::BTreeMap::new();

    let from_td_fps = fps_of(d, t, "fps")?;
    let fps = match cli_fps.or(from_td_fps) {
        Some(v) => {
            sources.insert(
                "fps".to_string(),
                if cli_fps.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("fps".to_string(), "内置默认");
            base.fps
        }
    };

    let from_td_canvas = canvas_of(d, t, "canvas")?;
    let canvas = match cli_canvas.or(from_td_canvas) {
        Some(v) => {
            sources.insert(
                "canvas".to_string(),
                if cli_canvas.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("canvas".to_string(), "内置默认");
            base.canvas
        }
    };

    let from_td_level = level_of(d, t, "level")?;
    let level = match cli_level.or(from_td_level) {
        Some(v) => {
            sources.insert(
                "level".to_string(),
                if cli_level.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("level".to_string(), "内置默认");
            base.level
        }
    };

    let from_td_font = s(d, t, "default_font");
    let default_font = match (cli_font, from_td_font) {
        (Some(v), _) => {
            sources.insert("default_font".to_string(), "命令行");
            v.to_string()
        }
        (None, Some(v)) => {
            sources.insert("default_font".to_string(), "tool-defaults");
            v
        }
        (None, None) => {
            sources.insert("default_font".to_string(), "内置默认");
            base.default_font.clone()
        }
    };

    Ok(Sourced {
        value: PgsEncodeDefaults {
            fps,
            canvas,
            level,
            default_font,
        },
        sources,
    })
}

/// 求 `render` 的最终取值。`exact_colors` 不在里面：它是个开关，
/// 不是"默认值"这种可以跨会话保存的偏好（开关的语义是"这次要不要"）。
pub fn render(
    d: &td::ToolDefaults,
    cli_fps: Option<Fps>,
    cli_canvas: Option<Canvas>,
    cli_font: Option<&str>,
    cli_min_axis: Option<u32>,
) -> Result<Sourced<RenderDefaults>> {
    let base = RenderDefaults::default();
    let t = Tool::Render;
    let mut sources = std::collections::BTreeMap::new();

    let from_td_fps = fps_of(d, t, "fps")?;
    let fps = match cli_fps.or(from_td_fps) {
        Some(v) => {
            sources.insert(
                "fps".to_string(),
                if cli_fps.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("fps".to_string(), "内置默认");
            base.fps
        }
    };

    let from_td_canvas = canvas_of(d, t, "canvas")?;
    let canvas = match cli_canvas.or(from_td_canvas) {
        Some(v) => {
            sources.insert(
                "canvas".to_string(),
                if cli_canvas.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("canvas".to_string(), "内置默认");
            base.canvas
        }
    };

    let from_td_font = s(d, t, "default_font");
    let default_font = match (cli_font, from_td_font) {
        (Some(v), _) => {
            sources.insert("default_font".to_string(), "命令行");
            v.to_string()
        }
        (None, Some(v)) => {
            sources.insert("default_font".to_string(), "tool-defaults");
            v
        }
        (None, None) => {
            sources.insert("default_font".to_string(), "内置默认");
            base.default_font.clone()
        }
    };

    let from_td_min = u32_of(d, t, "min_axis")?;
    let min_axis = match cli_min_axis.or(from_td_min) {
        Some(v) => {
            sources.insert(
                "min_axis".to_string(),
                if cli_min_axis.is_some() {
                    "命令行"
                } else {
                    "tool-defaults"
                },
            );
            v
        }
        None => {
            sources.insert("min_axis".to_string(), "内置默认");
            base.min_axis
        }
    };

    Ok(Sourced {
        value: RenderDefaults {
            fps,
            canvas,
            default_font,
            min_axis,
        },
        sources,
    })
}

/// 求 `font subset` 的 `fallback_full` 与 `subset_dir`。
///
/// `exact_colors`（`pgs encode`）与 `embed_fonts` 这类**开关**不在这里：
/// 开关的语义是"这次要不要做这件事"，而不是"过去我偏好什么"。
pub fn font_subset(d: &td::ToolDefaults) -> Result<(bool, Option<std::path::PathBuf>)> {
    let t = Tool::FontSubset;
    let fallback_full = bool_of(d, t, "fallback_full")?.unwrap_or(false);
    let subset_dir = s(d, t, "subset_dir").map(std::path::PathBuf::from);
    Ok((fallback_full, subset_dir))
}

/// 求 `mkv` 的 `tools_dir`。
pub fn mkv(d: &td::ToolDefaults) -> Option<std::path::PathBuf> {
    s(d, Tool::Mkv, "tools_dir").map(std::path::PathBuf::from)
}

/// 读一次 ToolDefaults。执行阶段的每个命令都该用这个，而不是各自构造 `DefaultsStore`。
pub fn load() -> td::DefaultsLoad {
    // `STK_TOOL_DEFAULTS` 优先：与 `STK_SETTINGS` 同一理由
    // （测试与"把设置放在别处"都要一条不依赖 exe 位置的路）。
    match std::env::var("STK_TOOL_DEFAULTS") {
        Ok(p) if !p.trim().is_empty() => td::DefaultsStore::at(std::path::PathBuf::from(p)).load(),
        _ => td::DefaultsStore::current().load(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn td_with(tool: &str, pairs: &[(&str, serde_json::Value)]) -> td::ToolDefaults {
        let mut d = td::ToolDefaults::default();
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        d.tools.insert(tool.to_string(), m);
        d
    }

    // ---- 优先级：用户值 > ToolDefaults > 内置默认 ----

    #[test]
    fn with_nothing_set_we_get_the_builtin_defaults() {
        let d = td::ToolDefaults::default();
        let r = pgs_encode(&d, None, None, None, None).unwrap();
        assert_eq!(r.value, PgsEncodeDefaults::default());
        for k in ["fps", "canvas", "level", "default_font"] {
            assert_eq!(r.sources[k], "内置默认", "{k} 的来源不对");
        }
    }

    #[test]
    fn tool_defaults_beat_the_builtin_default() {
        let d = td_with(
            "pgs encode",
            &[
                ("level", serde_json::json!("basic")),
                ("canvas", serde_json::json!("1280x720")),
            ],
        );
        let r = pgs_encode(&d, None, None, None, None).unwrap();
        assert_eq!(r.value.level, ValidationLevel::Basic);
        assert_eq!(r.value.canvas, Canvas::HD720);
        assert_eq!(r.sources["level"], "tool-defaults");
        assert_eq!(r.sources["canvas"], "tool-defaults");
        // 没设的那两项仍然来自内置默认
        assert_eq!(r.sources["fps"], "内置默认");
    }

    #[test]
    fn the_command_line_beats_tool_defaults() {
        // 命令行是当下的意图，设置文件是过去的偏好。
        let d = td_with("pgs encode", &[("level", serde_json::json!("basic"))]);
        let r = pgs_encode(&d, None, None, Some(ValidationLevel::Strict), None).unwrap();
        assert_eq!(r.value.level, ValidationLevel::Strict);
        assert_eq!(r.sources["level"], "命令行");
    }

    #[test]
    fn tool_defaults_do_not_leak_between_tools() {
        let d = td_with("render", &[("canvas", serde_json::json!("1280x720"))]);
        let r = pgs_encode(&d, None, None, None, None).unwrap();
        assert_eq!(
            r.value.canvas,
            Canvas::FHD1080,
            "render 的画布漏到了 pgs encode"
        );
    }

    // ---- 值不合法时必须报错，不能静默退回默认值 ----

    #[test]
    fn a_bad_level_is_an_error_not_a_silent_fallback() {
        // 静默退回会让用户以为自己的设置生效了，而他其实在用别的值。
        let d = td_with("pgs encode", &[("level", serde_json::json!("standardd"))]);
        let e = pgs_encode(&d, None, None, None, None).unwrap_err();
        assert!(e.title.contains("校验级别"), "{}", e.title);
        assert!(
            e.user_action
                .as_deref()
                .unwrap_or("")
                .contains("tool-defaults set"),
            "改法要指向设置命令: {:?}",
            e.user_action
        );
    }

    #[test]
    fn a_bad_canvas_is_an_error() {
        let d = td_with("pgs encode", &[("canvas", serde_json::json!("1920*1080"))]);
        assert!(pgs_encode(&d, None, None, None, None).is_err());
    }

    #[test]
    fn a_bad_fps_is_an_error() {
        let d = td_with("pgs encode", &[("fps", serde_json::json!("abc"))]);
        assert!(pgs_encode(&d, None, None, None, None).is_err());
    }

    #[test]
    fn a_bad_u32_is_an_error() {
        let d = td_with("render", &[("min_axis", serde_json::json!("big"))]);
        assert!(render(&d, None, None, None, None).is_err());
    }

    #[test]
    fn a_bad_bool_is_an_error() {
        let d = td_with(
            "font subset",
            &[("fallback_full", serde_json::json!("yes"))],
        );
        assert!(font_subset(&d).is_err());
    }

    // ---- 格式的宽容度：该宽的宽，该严的严 ----

    #[test]
    fn fps_accepts_both_a_label_and_a_rational() {
        for (written, want) in [("23.976", Fps::NTSC_23976), ("24000/1001", Fps::NTSC_23976)] {
            let d = td_with("render", &[("fps", serde_json::json!(written))]);
            let r = render(&d, None, None, None, None).unwrap();
            assert_eq!(r.value.fps, want, "{written} 没解析成 {want:?}");
        }
    }

    #[test]
    fn a_numeric_json_value_and_a_string_both_work_for_min_axis() {
        // 设置文件里的值可能是 JSON 数字（`set` 写的）也可能被手工改成字符串。
        for v in [serde_json::json!(8), serde_json::json!("8")] {
            let d = td_with("render", &[("min_axis", v.clone())]);
            let r = render(&d, None, None, None, None).unwrap();
            assert_eq!(r.value.min_axis, 8, "{v} 没解析成 8");
        }
    }

    #[test]
    fn canvas_accepts_uppercase_x() {
        let d = td_with("render", &[("canvas", serde_json::json!("1920X1080"))]);
        assert_eq!(
            render(&d, None, None, None, None).unwrap().value.canvas,
            Canvas::FHD1080
        );
    }

    // ---- render 侧 ----

    #[test]
    fn render_min_axis_comes_from_tool_defaults() {
        let d = td_with("render", &[("min_axis", serde_json::json!(8))]);
        let r = render(&d, None, None, None, None).unwrap();
        assert_eq!(r.value.min_axis, 8);
        assert_eq!(r.sources["min_axis"], "tool-defaults");
    }

    #[test]
    fn render_min_axis_zero_is_a_real_value_not_absent() {
        // 0 是"不过滤"，是**一个取值**而不是"没设置"。
        // 这里钉住它不被当成 None 而被设置里的值盖掉。
        let d = td_with("render", &[("min_axis", serde_json::json!(8))]);
        let r = render(&d, None, None, None, Some(0)).unwrap();
        assert_eq!(r.value.min_axis, 0, "命令行给的 0 被设置盖掉了");
        assert_eq!(r.sources["min_axis"], "命令行");
    }

    // ---- font subset / mkv ----

    #[test]
    fn font_subset_reads_both_keys() {
        let d = td_with("font subset", &[("fallback_full", serde_json::json!(true))]);
        let (fb, dir) = font_subset(&d).unwrap();
        assert!(fb);
        assert_eq!(dir, None);
    }

    #[test]
    fn mkv_tools_dir_is_read() {
        let d = td_with("mkv", &[("tools_dir", serde_json::json!("D:\\tools"))]);
        assert_eq!(
            mkv(&d),
            Some(std::path::PathBuf::from("D:\\tools")),
            "路径原样取用，不做规范化"
        );
    }
}
