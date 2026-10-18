//! 36 §1 第三层 Preset 的 CLI 面：`preset show|validate|example`。
//!
//! ## 三个动作各自回答一个问题
//!
//! | 动作 | 回答 | 典型调用者 |
//! | --- | --- | --- |
//! | `show` | 里面是什么？ | 我自己，想知道这份配置会产出什么 |
//! | `validate` | 能不能用？ | CI / 提交前检查，只要一个退出码 |
//! | `example` | 我该照什么写？ | 第一次写 preset 的人 |
//!
//! 把它们合成一个 `show` 会让 CI 被迫去解析人的输出 —— 那是一条
//! 又脆又没必要的关系。
//!
//! ## 为什么 `show` 要报 "problems" 而不仅是内容
//!
//! 因为 preset 里有几类东西**不一定是错但一定得说**：
//! 读不懂的陌生字段（会保留但不参与执行）、超范围的画布、未来版本的 schema。
//! 只说"读到了 2 个目标"会让用户以为自己看到了全部。
//!
//! ## 为什么退出码与 `show` 无关
//!
//! `show` **只要文件能读就退 0**，哪怕它有 problems —— 用户就是想看看里面
//! 有什么、哪里有毛病。`validate` 则在有 problems 时退非零。
//! 两者用同一个解析器，但**判断"是否成功"的标准不同**，
//! 因为它们的用途不同。

use std::path::Path;

use core_model::preset as p;
use serde_json::json;

use crate::args::PresetAction;
use crate::output::Envelope;

/// 规格 §2 的样板。**逐字**来自规格，不在这里另写一份。
///
/// 如果这份和规格里的示例出现分歧，那就是文档错了或代码错了 ——
/// 两者必有一错，而不该悄悄允许两份同时存在。
/// `preset::tests::the_spec_example_parses_as_written` 锁住了这一点：
/// 这段文本必须能被自己的解析器读出来。
pub const EXAMPLE: &str = r#"schema = 1
name = "1080p ASS + Basic PGS"

[[outputs]]
name = "1080p-ASS"
format = "ass"
canvas = [1920, 1080]
fps = "24000/1001"
hdr.mode = "off"
suffix = ".1080p"

[[outputs]]
name = "1080p-Basic-SUP"
format = "pgs"
canvas = [1920, 1080]
fps = "24000/1001"
renderer = "libass"
pgs.validation = "basic"
suffix = ".1080p.basic"
"#;

pub fn preset_cmd(
    action: PresetAction,
    path: Option<&Path>,
    cmd: &str,
) -> core_model::error::Result<Envelope> {
    match action {
        PresetAction::Example => {
            // 样板**只打样板本身**，不打 JSON 信封。
            //
            // 第一版把两个都打了，实测立刻出事：
            //   `preset example > my.toml` 存出来的文件里前半是 TOML、
            //   后半是 JSON，再拿回来 `validate` 就报“第 20 行 key 没有值”。
            // 而 `example` 的**全部用途**就是“原样拿走存成文件”——
            // 把别的东西混进 stdout 会让它每次都失败。
            //
            // 信息照样给足：样板本身就是完整的说明。
            // 如果真的需要机器可读的成功信号，看**退出码**（0 = 成功）。
            print!("{EXAMPLE}");
            Ok(Envelope::ok_quiet(cmd))
        }
        PresetAction::Show => {
            let path = path.expect("show 需要路径（参数解析已保证）");
            let load = p::load_file(path)?;
            let targets: Vec<serde_json::Value> = load
                .preset
                .outputs
                .iter()
                .map(|o| {
                    json!({
                        "name": o.name,
                        "format": o.format,
                        "canvas": o.canvas,
                        "fps": o.fps,
                        "hdr_mode": o.hdr.as_ref().and_then(|h| h.mode.clone()),
                        "suffix": p::effective_suffix(o),
                        "suffix_is_explicit": o.suffix.is_some(),
                        "renderer": o.renderer,
                        "pgs_validation": o.pgs.as_ref().and_then(|g| g.validation.clone()),
                    })
                })
                .collect();
            let collisions = p::suffix_collisions(&load.preset);
            let mut problems = load.problems.clone();
            problems.extend(collisions);
            let env = Envelope::ok(
                cmd,
                json!({
                    "path": path.display().to_string(),
                    "schema": load.preset.schema,
                    "name": load.preset.name,
                    "outputs": targets,
                    // 读不懂的字段**单独列出来**：它们被保留了（36 §3），
                    // 但“留着”与“生效”是两件事，混在一起会让用户以为它生效了。
                    "preserved_unknown_fields":
                        load.preset.unknown.keys().collect::<Vec<_>>(),
                    "problems": problems,
                }),
            );
            Ok(env)
        }
        PresetAction::Validate => {
            let path = path.expect("validate 需要路径（参数解析已保证）");
            // 这里的失败必须是**带原因的**失败，而不是一句"校验不通过"。
            // 调用方是 CI，它要把原因打进日志。
            let load = p::load_file(path)?;
            let collisions = p::suffix_collisions(&load.preset);
            let mut problems = load.problems.clone();
            problems.extend(collisions);
            if problems.is_empty() {
                Ok(Envelope::ok(
                    cmd,
                    json!({
                        "path": path.display().to_string(),
                        "outputs": load.preset.outputs.len(),
                        "problems": Vec::<String>::new(),
                    }),
                ))
            } else {
                // 有 problems 就是**校验失败** —— 这不是"读不了"（那会走
                // load_file 的 Err），而是"读得了但不该就这么用"。
                // 两者的区别对 CI 没有意义（都要红），但对读日志的人有意义。
                Err(core_model::error::StkError::simple(
                    p::codes::DOMAIN,
                    p::codes::BAD_PRESET_NO,
                    p::codes::BAD_PRESET.1,
                )
                .with_detail(format!("{}：{}", path.display(), problems.join("；")))
                .with_action("用 `preset show` 看完整内容与每个问题的上下文"))
            }
        }
    }
}
