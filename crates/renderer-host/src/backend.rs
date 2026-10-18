//! 宿主侧的 **provider 分派**：把"用哪条链"这件事从渲染循环里抽出来。
//!
//! ## 为什么需要这一层
//!
//! 12 §1 说 renderer 有**三条互相独立**的能力链。`main.rs` 原来直接写
//! `render_libass::Provider`，于是"换一种 provider"意味着改渲染循环 ——
//! 而渲染循环里还混着协议、崩溃追踪、帧分块那些与 provider 无关的东西。
//!
//! ## 一条硬规矩：不许"先试 A 失败再试 B"
//!
//! 分派必须**先看导出表**（12 §1），然后**只走一条链**。
//!
//! 试错式分派在这里特别坏：`LoadLibrary` 会在进程里留下已初始化的
//! provider 状态，而"加载到一半发现不对"留下的半初始化状态没有任何
//! 办法清干净。那正是 `renderer-host` 要用进程隔离避免的东西 ——
//! 我们不该在**隔离进程内部**再制造一次同样的麻烦。
//!
//! ## 能力差异要**如实报告**，不是抹平
//!
//! 两条链的能力不同，最明显的一项：**CSRI 头文件里没有加字体的入口**，
//! 所以"把内存字体交给渲染器"（17 §7）在 CSRI 链上**做不到**。
//!
//! 这一条不能靠"静默忽略"过去：用户以为字体注入了、而实际用的是系统字体，
//! 结果是**渲染出来的字形和预期不同却不报警**。
//! 所以 [`Backend::supports_font_injection`] 返回 `false`，
//! 而调用方必须把这件事说出去。

use core_model::canvas::Canvas;
use core_model::error::Result;
use core_model::time::Time;
use render_api::capability::ProviderKind;

/// 一次渲染的产物（provider 无关）。
///
/// 与 `render_libass::Rendered` 的差别：这里只有**像素与元数据**，
/// 没有 libass 特有的图像链。两条链都要能填出这个结构。
pub struct RawFrame {
    pub frame: image_core::Frame,
    /// 1=位置可能变了，2=内容可能变了（借 libass 的 detect_change 语义）
    pub detect_change: i32,
    /// 这一帧由多少个图元合成（libass 有图像链；CSRI 只有一张位图，记 1 或 0）
    pub images: usize,
    pub loss: core_model::loss::LossReport,
}

/// 已经打开的 provider，以及它属于哪条链。
pub enum Backend {
    /// libass 链。脚本**不在**这里 —— 它由调用方持有（`Script` 借用 provider）。
    Libass(Box<render_libass::Provider>),
    /// CSRI 链。脚本在 `open` 时就交进去了，所以这里自己持有。
    Csri(Box<render_csri::CsriProvider>),
}

impl Backend {
    /// 打开一个 provider。**先看导出表，再加载**（12 §1/§3）。
    ///
    /// `script` 是 ASS 文本。CSRI 那条链在打开时就要把脚本交进去
    /// （`csri_open_mem`），而 libass 是分开两步（`load_script`）——
    /// 这个差异由本函数吸收，调用方不必知道。
    pub fn open(
        dll: &std::path::Path,
        script: &str,
        storage: Canvas,
        output: Canvas,
        default_font: &str,
    ) -> Result<Backend> {
        let kind = classify(dll)?;
        match kind {
            ProviderKind::Csri => {
                // 12 §1B 的链：脚本在 open 时就交进去。
                //
                // 注意**不传文件路径**：我们已经把脚本读进来并校验过，
                // 再让 provider 去读一次盘会把"我们校验过的那份"与
                // "它读的那份"变成两个可能不同的东西。
                // 只在 provider 不支持内存入口时才会用到路径 —— 那种
                // 情况下它会明确报 STK-RND-0092，而不是默默读盘。
                let p = render_csri::CsriProvider::open(
                    dll,
                    script,
                    None,
                    output.width,
                    output.height,
                )?;
                Ok(Backend::Csri(Box::new(p)))
            }
            _ => {
                let p = render_libass::Provider::open(dll, storage, output, default_font)?;
                Ok(Backend::Libass(Box::new(p)))
            }
        }
    }

    /// 这条链的名字（进握手与报告）。
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Libass(_) => "libass",
            Backend::Csri(_) => "csri",
        }
    }

    /// provider 自己的版本串（拿不到就给空）。
    pub fn version(&self) -> String {
        match self {
            Backend::Libass(p) => format!("0x{:08X}", p.library_version()),
            Backend::Csri(p) => {
                // CSRI 没有"库版本"这个统一入口；`csri_renderer_info()` 的
                // `longname` 最接近（实测本机给的是 "2.39"）。
                p.report.longname.clone()
            }
        }
    }

    /// **这条链能不能接受内存字体**（17 §7）。
    ///
    /// - libass：能（`ass_add_font`）。
    /// - CSRI：**不能** —— 头文件里没有加字体的入口。
    ///
    /// 返回 `false` 时调用方**必须**把"没注入"这件事报出去，
    /// 不能静默忽略：用户会以为字形来自他给的那份字体。
    pub fn supports_font_injection(&self) -> bool {
        matches!(self, Backend::Libass(_))
    }

    /// 交一份内存字体给渲染器。
    ///
    /// 对不支持的链返回 `Ok(0)` **不表示成功** —— 它表示"没做"。
    /// 调用方要先看 [`Self::supports_font_injection`]。
    /// 之所以还返回 `Ok` 而不是 `Err`：这不是失败，是能力缺失，
    /// 而把它做成错误会让"某份 ASS 带内嵌字体"在 CSRI 链上整体渲染不了。
    /// **判断该由调用方按能力做，而不是靠错误码去猜。**
    pub fn add_font(&mut self, family: &str, bytes: &[u8]) -> Result<()> {
        match self {
            Backend::Libass(p) => p.add_font(family, bytes),
            // 明确“没做”。返回 `Ok(())` **不表示成功** ——
            // 调用方必须先看 [`Self::supports_font_injection`]。
            //
            // 为什么不做成 `Err`：这不是失败，是**能力缺失**。
            // 做成错误会让“某份 ASS 带内嵌字体”在 CSRI 链上整体渲染不了，
            // 而那比“没注入”坏得多 —— 用户还能拿到字幕，只是字形未必是他要的。
            Backend::Csri(_) => Ok(()),
        }
    }

    /// 渲染某个时刻的一帧。
    ///
    /// `libass_script` 只在 libass 链上有意义：那条链的 `Script` 借用了
    /// provider 的渲染器句柄，所以脚本必须由**调用方**持有并传进来。
    /// CSRI 链不用它（脚本已经在 `open` 时交进去了）。
    pub fn render_at(
        &mut self,
        t: Time,
        output: Canvas,
        libass_script: Option<&render_libass::Script>,
    ) -> Result<RawFrame> {
        match self {
            Backend::Libass(_) => {
                let script = libass_script.ok_or_else(|| {
                    core_model::error::StkError::simple(
                        core_model::error::Domain::Render,
                        99,
                        "还没加载脚本就要求出帧",
                    )
                })?;
                let r = render_libass::render(script, t, output)?;
                Ok(RawFrame {
                    frame: r.frame,
                    detect_change: r.detect_change,
                    images: r.images,
                    loss: r.loss,
                })
            }
            Backend::Csri(p) => {
                let ms = whole_millis(t)?;
                let rgba = p.render_at(ms as f64 / 1000.0)?;
                let canvas = Canvas::new(rgba.width, rgba.height)?;

                // ## 为什么必须把“不透明的背景”剥掉
                //
                // CSRI provider 给的是**一整张已经合成好的位图**：
                // 字幕所在的部分是内容，其余地方是它自己填的背景
                // （实测本机 VSFilterMod 填的是不透明的黑）。
                //
                // 而下游（`image_core::Frame::bbox`）是**按 alpha 判定可见内容**的：
                // alpha != 0 的像素算“有东西”。于是一张带不透明背景的位图
                // 会被看成**整幅画布都有内容** —— 即使这一刻根本没有字幕。
                //
                // 实际撞上的后果：`front.rs` 末尾会补一个“空白采样”
                // （`Frame::empty`，全透明），用来让最后一个事件结束。
                // 而 CSRI 那侧在这个时刻给的是“全不透明黑”，
                // 于是空自采样与非空实际采样**不相等**，产出一个
                // 长度为零的事件，BDN 把它报成 `STK-BDN-0059 事件结束不晚于开始`。
                //
                // 所以这里把“不透明”剥成“透明”：
                // **alpha 保留原值，只看 RGB 是否与背景同色**是做不到的
                // （字幕本身也可能是黑的）。可靠的做法是问 provider
                // “这一帧里哪些像素是字幕”——而 CSRI 没有那个接口。
                //
                // 能确认的只有一件事：**整张不透明的位图里，没有内容的地方是背景**。
                // 而“背景”在 ASS 渲染的语义里就是“不该被交付出去的东西” ——
                // 我们的产物（BDN/SUP）是**带 alpha 的字幕层**，不是带底的成片。
                //
                // 因此按“与背景色相同 → 置为全透明”来剥离，并把背景色记为
                // 采样到的左上角像素（provider 给的第一张不透明位图的角点）。
                let mut loss = core_model::loss::LossReport::new();
                let mut pixels = rgba.pixels;
                let blank_before = pixels.iter().all(|b| *b == 0);
                let knocked_out = knock_out_background(&mut pixels, &mut loss);
                let frame = image_core::Frame {
                    canvas,
                    rgba: pixels,
                };

                // CSRI 没有 `detect_change` 这种“内容变没变”的输出；
                // 它给的是整张合成位图。**不能瞎填 2（可能变了）** ——
                // 那会让调用方以为每次都有变化，从而每帧都算新帧。
                // 填 0 表示“未报告”，让调用方自己按像素比。
                loss.user_attention(
                    "CSRI 链不报告 detect_change：它给的是整张合成位图，\
                     变化检测由下游按像素比（12 §1B）",
                );
                if knocked_out {
                    loss.preserved(
                        "CSRI 背景已按“与角点同色→透明”剥离：下游按 alpha 判定可见内容，\
                         留着不透明背景会被当成整幅画布都有内容（12 §1B）",
                    );
                }
                Ok(RawFrame {
                    frame,
                    detect_change: 0,
                    images: usize::from(!blank_before),
                    loss,
                })
            }
        }
    }
}

/// 把 CSRI 合成位图里的**背景**剥成透明。
///
/// 返回是否真的敲掉了东西（用于记账）。见调用点那段长注释里为什么必须做这件事。
///
/// ## 判据为什么是“与角点同色”而不是“黑色”
///
/// 把“黑”当背景是错的：字幕本身常常就是黑的（黑边、黑字），
/// 而那样会被一并敲掉。角点（0,0）在字幕布局里几乎总在文字之外 ——
/// ASS 的对齐惯例与边距让文字很少压到画布左上角。
///
/// 这不是完美判据，所以它只做**一件事、且可观察**：
/// 敲掉了多少就记多少，不静默。真正的修法是让 provider 交出预乘 alpha 的字幕层，
/// 而 CSRI 头文件里没有那个入口。
fn knock_out_background(pixels: &mut [u8], loss: &mut core_model::loss::LossReport) -> bool {
    if pixels.len() < 4 {
        return false;
    }
    let bg = [pixels[0], pixels[1], pixels[2]];
    let mut hit = 0usize;
    // 用 `chunks_mut(4)` 而不是 `chunks_exact_mut(4)`：
    // 后者要求长度是 4 的倍数，而这里只需“按 4 分组、尾数忽略”。
    for px in pixels.chunks_mut(4) {
        // 只动“不透明且与背景同色”的像素：半透明的边缘像素保留，
        // 因为那是抗锯齿的过渡，敲掉会让边缘变硬。
        if px.len() == 4 && px[3] == 255 && px[0] == bg[0] && px[1] == bg[1] && px[2] == bg[2] {
            px[3] = 0;
            hit += 1;
        }
    }
    let total = pixels.len() / 4;
    // 只在“敲掉的比例不小”时记账：一两个同色像素是噪声。
    let meaningful = hit * 100 > total;
    if meaningful {
        loss.preserved(&format!(
            "CSRI 背景剥离：{hit}/{total} 个不透明像素与角点同色（{:?}），已置为透明",
            bg
        ));
    }
    meaningful
}

/// 按导出表判定一个 DLL 属于哪条链。**不加载**。
///
/// 抽样逻辑与 `render-csri::CsriProvider::probe` / `render-libass::probe`
/// 同源 —— 都走 `render-api::capability::probe` 与
/// `core_model::pe_exports_have_libass_required`，不各写一份。
pub fn classify(dll: &std::path::Path) -> Result<ProviderKind> {
    use core_model::error::{Domain, StkError};
    let exports = core_model::pe::exports_of_file(dll).map_err(|e| {
        // 100：与 `render-libass` 的“候选 DLL 不是有效的 PE 文件”(72) 分开。
        //
        // 两处描述的是**同一类事**（都不是 PE），但根因陈述不同：
        // 那边是“我要 libass，而这个文件不像”，这里是“我要分派，
        // 而连导出表都读不出来”。跨 crate 唯一性测试要求
        // “共享就统一标题”，而这两个标题确实不同 —— 那就分开编号。
        StkError::simple(Domain::Render, 100, "无法读候选 DLL 的导出表")
            .with_detail(format!("{}：{e}", dll.display()))
    })?;
    if core_model::pe_exports_have_libass_required(&exports.names) {
        return Ok(ProviderKind::LibassBuiltin);
    }
    let probe = render_api::capability::probe(
        dll.file_name().and_then(|s| s.to_str()).unwrap_or(""),
        &exports.names,
    );
    probe.kind.ok_or_else(|| {
        StkError::simple(Domain::Render, 96, "这个 DLL 不属于任何已知的 renderer 链")
            .with_detail(format!(
                "{}：既不是 libass，也不是 CSRI / VapourSynth / AviSynth",
                dll.display()
            ))
            .with_action("用 `provider probe <dll>` 看它到底是什么")
    })
}

fn whole_millis(t: Time) -> Result<i64> {
    let r = t.as_rational();
    // 与 libass 那条链同一口径：向上取整会让"结束那一帧仍然画得出来"，
    // 向下取整会悄悄截掉尾巴。
    let n = r.num();
    let d = r.den().max(1);
    // `checked_*`：恶意/极端输入不该在乘法上回绕。
    let ms = n
        .checked_mul(1000)
        .and_then(|v| v.checked_add(d - 1))
        .map(|v| v / d)
        .ok_or_else(|| {
            // 98：不拿 3 —— 那是 `render-api` 的“非法比率”，两件事不同。
            core_model::error::StkError::simple(
                core_model::error::Domain::Render,
                98,
                "时间超出能表达的毫秒范围",
            )
        })?;
    Ok(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_millis_rounds_up_like_the_libass_chain() {
        assert_eq!(whole_millis(Time::ZERO).unwrap(), 0);
        assert_eq!(
            whole_millis(Time::from_millis(1500).unwrap()).unwrap(),
            1500
        );
        // 半毫秒向上 → 1（向下会截掉尾巴）
        let half = Time::from_millis(1).unwrap();
        assert_eq!(whole_millis(half).unwrap(), 1);
    }

    /// 分类器对**不存在的文件**要给出可读原因，而不是 panic。
    #[test]
    fn classifying_a_missing_file_is_a_named_error() {
        let e = classify(std::path::Path::new("Z:/nope/not-here.dll")).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0100");
    }

    /// 分类器对**不是 PE** 的文件同样。
    #[test]
    fn classifying_a_non_pe_file_is_a_named_error() {
        let p = std::env::temp_dir().join(format!("stk-notpe-{}.dll", std::process::id()));
        std::fs::write(&p, b"definitely not a PE").unwrap();
        let e = classify(&p).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0100");
        let _ = std::fs::remove_file(&p);
    }
}
