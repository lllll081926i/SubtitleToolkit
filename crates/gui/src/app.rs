use crate::message::Message;
use crate::state::{InputsState, PreviewState, ProfilesState, TaskDrawerState};
use crate::views::{header, inputs_view, preview_view, profiles_view, task_view};
use iced::widget::{column, container, row};
use iced::{Element, Length, Task, Theme};

pub struct SubtitleApp {
    pub inputs: InputsState,
    pub preview: PreviewState,
    pub profiles: ProfilesState,
    pub task_drawer: TaskDrawerState,
    pub show_doctor: bool,
    /// 最近一次环境自检结果（`None` = 还没跑过）
    pub doctor: Option<Result<serde_json::Value, String>>,
    /// 当前任务的取消令牌。
    ///
    /// 以前这个标志是在后台闭包里建的，UI 根本拿不到，于是“取消”按钮
    /// 只改了一个 bool 就什么都不做——点下去没有任何效果。
    /// 令牌必须存在能被 UI 碰到的位置（AT-UI-001：取消可点击且**真的**生效）。
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Default for SubtitleApp {
    fn default() -> Self {
        Self {
            inputs: InputsState::new(),
            preview: PreviewState::new(),
            profiles: ProfilesState::new(),
            task_drawer: TaskDrawerState::new(),
            show_doctor: false,
            doctor: None,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl SubtitleApp {
    pub fn new() -> (Self, Task<Message>) {
        (Self::default(), Task::none())
    }

    pub fn title(&self) -> String {
        "SubtitleToolkit-RS 字幕工程工作台".into()
    }

    /// 请求渲染当前预览帧。
    ///
    /// 三个入口（选中输入 / 改时间 / 切 A/B）共用它：
    /// “什么时候重渲”只能有一处判断，否则“改了时间但忘了重渲”这类洞
    /// 会在某一个入口上单独出现（那就意味着换一条路进来看到的还是旧帧）。
    ///
    /// A/B 开关打开时请求**两张**帧（05 §4.1）：左半是原版，右半是应用了
    /// 当前 Profile 变换的版本。变换用的是 `transform_hdr` / `transform_resolution`
    /// ——与 `hdr` 命令和导出链同一份实现，所以对比结果就是导出的预演。
    fn request_preview(&mut self) -> Task<Message> {
        let Some(item) = self.inputs.active_input() else {
            return Task::none();
        };
        let path = item.path.clone();
        let time = self.preview.current_time_ms;
        self.preview.is_rendering = true;

        // 画布：用当前选中 Profile 的目标尺寸作为 B 侧；A 侧用同一存储画布。
        //
        // A 侧刻意**不**跟着 Profile 的尺寸走：规格写的是“左半屏显示原版
        // （SDR / 原始尺寸）”。若让 A 也跟着目标画布，两个画面就只剩色调差异，
        // “缩放导致的版面变化”就看不到了 —— 而那是用户开对比的一半理由。
        let (canvas_a, canvas_b, hdr, has_hdr) = self.ab_plan();
        tracing::debug!(
            input = %path.file_name().and_then(|name| name.to_str()).unwrap_or(""),
            time_ms = time,
            ab = self.preview.is_split_ab,
            canvas_a = ?canvas_a,
            canvas_b = ?canvas_b,
            "请求预览渲染"
        );
        // 闭包要 `'static`：在它**外面**把两个画布拷成局部值，
        // 而不是靠在闭包里再取（那样会借到 `canvas_a` 本身）。
        let (ca_w, ca_h) = canvas_a;
        let (cb_w, cb_h) = canvas_b;

        if self.preview.is_split_ab {
            return Task::perform(
                async move {
                    crate::worker::preview_worker::render_ab(
                        crate::worker::preview_worker::AbRequest {
                            ass_path: path,
                            time_ms: time,
                            canvas_a: (ca_w, ca_h),
                            canvas_b: (cb_w, cb_h),
                            hdr,
                            scale_to_canvas: (ca_w, ca_h) != (cb_w, cb_h),
                        },
                    )
                    .await
                },
                move |res| match res {
                    Ok(f) => Message::AbRendered {
                        a_rgba: f.a.rgba,
                        a_dims: (f.a.width, f.a.height),
                        b_rgba: f.b.rgba,
                        b_dims: (f.b.width, f.b.height),
                        note: f.note,
                        source: f.b.source,
                    },
                    Err(e) => Message::AbRendered {
                        a_rgba: vec![],
                        a_dims: (ca_w, ca_h),
                        b_rgba: vec![],
                        b_dims: (cb_w, cb_h),
                        note: String::new(),
                        source: crate::state::preview::FrameSource::Failed { code: e },
                    },
                },
            );
        }

        // 非 A/B：单帧，尺寸用 A 侧（原版）。
        // 这里保留 `hdr`/`has_hdr` 的读取，它们只在 A/B 里用。
        let _ = (hdr, has_hdr);
        Task::perform(
            async move {
                crate::worker::preview_worker::render_single_frame(path, time, (ca_w, ca_h)).await
            },
            |res| match res {
                Ok(f) => Message::FrameRendered {
                    rgba: f.rgba,
                    width: f.width,
                    height: f.height,
                    source: f.source,
                },
                Err(e) => Message::FrameRendered {
                    rgba: vec![],
                    width: 1920,
                    height: 1080,
                    source: crate::state::preview::FrameSource::Failed { code: e },
                },
            },
        )
    }

    /// 从当前选中的 Profile 推出 A/B 两侧的画布与 HDR 设置。
    ///
    /// 没有选中 Profile 时退回“两侧相同、不做变换”——
    /// 那是一个**惰性但正确**的结果（两张一样的图 + 注记说明没做变换），
    /// 比编一个变换出来好。
    fn ab_plan(&self) -> ((u32, u32), (u32, u32), crate::state::profile::HdrMode, bool) {
        use crate::state::profile::CanvasMode;
        let original = (1920u32, 1080u32);
        let Some(p) = self.profiles.active_profile() else {
            return (
                original,
                original,
                crate::state::profile::HdrMode::Sdr,
                false,
            );
        };
        let target = match p.canvas {
            CanvasMode::KeepOriginal => original,
            CanvasMode::Hd720 => (1280, 720),
            CanvasMode::Fhd1080 => (1920, 1080),
            CanvasMode::Uhd2160 => (3840, 2160),
            CanvasMode::Custom(w, h) => (w, h),
        };
        let has_hdr = !matches!(p.hdr, crate::state::profile::HdrMode::Sdr);
        (original, target, p.hdr, has_hdr)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // === 左栏输入事件 ===
            Message::InputSelected(id) => {
                self.inputs.active_preview_id = Some(id);
                return self.request_preview();
            }
            Message::InputToggled(id) => self.inputs.toggle_select(id),
            Message::SelectAllInputs(s) => self.inputs.select_all(s),
            Message::RemoveInput(id) => self.inputs.remove_item(id),
            Message::AddFilesClicked => {
                let files = rfd::FileDialog::new()
                    .add_filter("字幕文件", &["ass", "ssa", "srt", "sup"])
                    .pick_files();
                if let Some(paths) = files {
                    return Task::perform(async move { paths }, Message::FilesDropped);
                }
            }
            Message::FilesDropped(paths) => {
                for p in paths {
                    self.inputs.add_path(p);
                }
            }
            Message::VideoAssistDropped(p) => {
                // 用真的 ffprobe 读（16 §/AT-PROBE-001）：
                // 这里曾经写死“建议帧率 23.976 | 分辨率 3840x2160”，
                // 那个数字对任何视频都“对”，所以没人会发现它从来没被测过。
                // 探不到就说探不到，并把 hint 给出来。
                let probe = crate::worker::probe_worker::probe_video(&p);
                self.inputs.video_assist = Some(probe.assist.clone());
                self.inputs.banner_suggestion = Some(probe.banner);
            }
            Message::ApplyVideoAssist => {
                if let Some(p) = self.profiles.active_profile_mut() {
                    p.canvas = crate::state::profile::CanvasMode::Uhd2160;
                }
                self.inputs.banner_suggestion = None;
            }
            Message::DismissVideoBanner => {
                self.inputs.banner_suggestion = None;
            }

            // === 中栏预览事件 ===
            Message::PreviewTimeChanged(time) => {
                self.preview.set_time(time);
                return self.request_preview();
            }
            Message::PreviewJumpEvent(next) => {
                let step = if next { 2000 } else { -2000 };
                let t = (self.preview.current_time_ms + step).max(0);
                return self.update(Message::PreviewTimeChanged(t));
            }
            Message::ToggleSplitAb => {
                self.preview.toggle_split_ab();
                // 05 §4.1：开关不是只换个标签 —— 打开时**真去渲对照帧**，
                // 关掉时把对照帧丢掉（留着它下次开时可能拿旧帧当新帧）。
                return self.request_preview();
            }
            Message::AbRendered {
                a_rgba,
                a_dims,
                b_rgba,
                b_dims,
                note,
                source,
            } => {
                self.preview.is_rendering = false;
                self.preview.ab_note = Some(note);
                if a_rgba.is_empty() || b_rgba.is_empty() {
                    let code = match source {
                        crate::state::preview::FrameSource::Failed { code } => code,
                        _ => "未提供错误码".to_string(),
                    };
                    tracing::error!(code = %code, "A/B 预览渲染失败");
                    self.preview.clear_frame(code);
                } else {
                    let provider = match &source {
                        crate::state::preview::FrameSource::Renderer { provider } => {
                            provider.clone()
                        }
                        other => other.label().to_string(),
                    };
                    self.preview
                        .set_ab_frames(a_rgba, a_dims, b_rgba, b_dims, provider);
                }
            }
            Message::FrameRendered {
                rgba,
                width,
                height,
                source,
            } => {
                if rgba.is_empty() {
                    // 渲染失败：清掉旧帧并如实标明。留着上一帧会让人以为
                    // “这就是现在这一刻的样子”，而那是上一刻的像素。
                    let code = match source {
                        crate::state::preview::FrameSource::Failed { code } => code,
                        _ => "未提供错误码".to_string(),
                    };
                    tracing::error!(code = %code, "单帧预览渲染失败");
                    self.preview.clear_frame(code);
                } else {
                    match source {
                        crate::state::preview::FrameSource::Renderer { provider } => self
                            .preview
                            .set_rendered_frame(rgba, width, height, provider),
                        crate::state::preview::FrameSource::Placeholder { reason } => self
                            .preview
                            .set_placeholder_frame(rgba, width, height, reason),
                        // 空 rgba 已经在上面处理；这里只是把"来源说不清"这一情形
                        // 如实归到占位，而不是冒充真实渲染。
                        other => self.preview.set_placeholder_frame(
                            rgba,
                            width,
                            height,
                            format!("像素来源无法确认（{}）", other.label()),
                        ),
                    }
                }
            }

            // === 右栏 Profile 矩阵事件 ===
            Message::ProfileSelected(id) => self.profiles.active_edit_id = Some(id),
            Message::ProfileToggled(id) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    p.enabled = !p.enabled;
                }
            }
            Message::AddProfile => self.profiles.add_profile(),
            Message::DuplicateProfile(id) => self.profiles.duplicate_profile(id),
            Message::RemoveProfile(id) => self.profiles.remove_profile(id),
            Message::FormatChanged(id, fmt) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    p.target_format = fmt;
                }
            }
            Message::CanvasChanged(id, c) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    p.canvas = c;
                }
            }
            Message::HdrChanged(id, h) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    p.hdr = h;
                }
            }
            Message::TimeAdjustChanged(id, t) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    p.time_adjust = t;
                }
            }

            // === 底栏任务抽屉事件 ===
            Message::StartQueue => {
                let inputs: Vec<(String, String)> = self
                    .inputs
                    .selected_inputs()
                    .into_iter()
                    .map(|i| (i.path.display().to_string(), i.file_name.clone()))
                    .collect();
                let profiles: Vec<_> = self
                    .profiles
                    .enabled_profiles()
                    .into_iter()
                    .cloned()
                    .collect();
                if inputs.is_empty() || profiles.is_empty() {
                    // 不给“看起来开始了”：什么都不摆，也不转圈
                    return Task::none();
                }
                let names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
                self.task_drawer.plan(&inputs, &names);
                self.task_drawer.is_running = true;
                self.task_drawer.is_expanded = true;

                // 编译成**真实的 batch spec**（15 §1），再交给 cli::batch::run_batch。
                // 这一步保证“GUI 复制的命令能在 CLI 重放”（20 §Repro）：
                // 两者用的是同一份 spec 类型、同一个执行入口。
                let roots: Vec<std::path::PathBuf> = inputs
                    .iter()
                    .map(|(p, _)| std::path::PathBuf::from(p))
                    .collect();
                let libass = std::env::var("STK_LIBASS_DLL")
                    .ok()
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_file());
                let (spec, notes) = crate::worker::spec::compile_spec(
                    &crate::worker::task_worker::job_id(),
                    &roots,
                    &profiles.iter().collect::<Vec<_>>(),
                    &self.task_drawer.output_dir,
                    libass,
                    "{stem}.{profile}.{ext}",
                );
                self.task_drawer.notes = notes.clone();
                let out_dir = self.task_drawer.output_dir.clone();
                // 把令牌换成全新的：上一任务可能已经把旧令牌置位，
                // 复用它会让新任务一开始就自认为被取消。
                self.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let cancel = self.cancel.clone();

                return Task::perform(
                    async move {
                        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                        // 真执行放在阻塞线程里：它要做文件 IO 与 CPU 渲染。
                        // 不能放在 async 任务里——那会把整个 UI 线程的运行时占住，
                        // 界面就不动了（AT-UI-001 要的是重任务期间仍然可响应）。
                        let handle = tokio::task::spawn_blocking(move || {
                            crate::worker::task_worker::run(&spec, &out_dir, cancel, |p| {
                                let _ = tx.send(p);
                            })
                        });
                        let mut updates = Vec::new();
                        let mut rx = rx;
                        while let Some(p) = rx.recv().await {
                            updates.push(p);
                        }
                        let result = handle.await.ok().and_then(|r| r.err());
                        (updates, result)
                    },
                    |(updates, err)| Message::BatchProduced {
                        updates,
                        error: err,
                    },
                );
            }
            Message::RetryFailed => {
                // AT-RENDER-002 的“GUI 可重试”。
                //
                // ## 为什么用 roots 而不是新增一个“只要这几对”的字段
                //
                // `Spec` 的语义是“roots 里的文件 × 全部 profile”（15 §1）。
                // 重试的集合是“失败的（输入, profile）” —— 那不一定是
                // 输入的全集 × profile 的全集。
                //
                // 两件事分开表达：
                //   - **哪些输入参加** → 只把有失败格的那些文件放进 `roots`；
                //   - **哪些 profile 参加** → 只把有失败格的那些 profile 放进去。
                // 两边的交集正好就是“失败的格子”，而不需要给 spec 加字段
                // （加字段会让这份 spec 不再能拿去 CLI 重放 —— 而“可重放”
                // 是 GUI 与 CLI 同源的全部价值）。
                let targets = self.task_drawer.retry_targets();
                if targets.is_empty() {
                    self.task_drawer
                        .notes
                        .push("没有可重试的格子（失败的与没跑到的都没有）".to_string());
                    return Task::none();
                }

                // 重试哪些 profile。用**名字**匹配，因为界面上的 profile
                // 可能在上一次执行之后被改过 —— 改过的按新参数跑，那是用户的意思。
                let want_profiles: Vec<&crate::state::profile::OutputProfile> = self
                    .profiles
                    .enabled_profiles()
                    .into_iter()
                    .filter(|p| targets.iter().any(|(_, n)| *n == p.name))
                    .collect();
                if want_profiles.is_empty() {
                    self.task_drawer.notes.push(
                        "要重试的 profile 现在已经不在启用列表里了 —— \
                         先把它们重新启用，或者直接点“开始”跑一批新的"
                            .to_string(),
                    );
                    return Task::none();
                }

                let want_inputs: Vec<std::path::PathBuf> = {
                    let mut v: Vec<std::path::PathBuf> = Vec::new();
                    for (path, _) in &targets {
                        let p = std::path::PathBuf::from(path);
                        if !v.contains(&p) {
                            v.push(p);
                        }
                    }
                    v
                };

                // 退回 Queued，否则统计会把上次的红格子继续算进去，
                // 界面会显示“跑完了，但还有 N 个失败”—— 那与“正在重试”矛盾。
                let n = self.task_drawer.reset_for_retry();
                self.task_drawer
                    .notes
                    .push(format!("开始重试 {n} 个格子（只跑失败与没跑到的）"));
                self.task_drawer.is_running = true;
                self.task_drawer.is_expanded = true;

                let libass = std::env::var("STK_LIBASS_DLL")
                    .ok()
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_file());
                let (spec, notes) = crate::worker::spec::compile_spec(
                    &crate::worker::task_worker::job_id(),
                    &want_inputs,
                    &want_profiles,
                    &self.task_drawer.output_dir,
                    libass,
                    "{stem}.{profile}.{ext}",
                );
                self.task_drawer.notes = notes;
                let out_dir = self.task_drawer.output_dir.clone();
                self.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let cancel = self.cancel.clone();

                return Task::perform(
                    async move {
                        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                        let handle = tokio::task::spawn_blocking(move || {
                            crate::worker::task_worker::run(&spec, &out_dir, cancel, |p| {
                                let _ = tx.send(p);
                            })
                        });
                        let mut updates = Vec::new();
                        let mut rx = rx;
                        while let Some(p) = rx.recv().await {
                            updates.push(p);
                        }
                        let result = handle.await.ok().and_then(|r| r.err());
                        (updates, result)
                    },
                    |(updates, err)| Message::BatchProduced {
                        updates,
                        error: err,
                    },
                );
            }
            Message::BatchProduced { updates, error } => {
                for u in updates {
                    match u {
                        crate::worker::task_worker::Progress::Started { spec_path, notes } => {
                            self.task_drawer.spec_path = Some(spec_path);
                            for n in notes {
                                if !self.task_drawer.notes.contains(&n) {
                                    self.task_drawer.notes.push(n);
                                }
                            }
                        }
                        crate::worker::task_worker::Progress::Row {
                            input,
                            profile,
                            status,
                            detail,
                        } => {
                            self.task_drawer.apply(
                                &input,
                                &profile,
                                crate::state::task::TaskStatus::from_report(&status, &detail),
                            );
                        }
                        crate::worker::task_worker::Progress::Finished {
                            outcome,
                            report_path,
                            error,
                        } => {
                            self.task_drawer.report_path = report_path;
                            let reason = error
                                .map(|(code, title, _)| format!("{code}: {title}"))
                                .unwrap_or_else(|| format!("任务判定为 {outcome}"));
                            // 自己算的判定放进抽屉：报告里的 outcome 与逐行状态
                            // 如果对不上，derived_outcome() 会把它暴露出来
                            self.task_drawer.finish(&outcome, &reason);
                        }
                    }
                }
                if let Some((code, title, action)) = error {
                    self.task_drawer
                        .finish("failed", &format!("{code}: {title}"));
                    self.task_drawer.notes.push(if action.is_empty() {
                        format!("任务失败：{code} {title}")
                    } else {
                        format!("任务失败：{code} {title}（{action}）")
                    });
                }
                // AT-RENDER-002：**失败之后一定要能重试**。
                //
                // 实测抳出的一个死角：有一种失败发生在**任何一行被报告之前** ——
                // 例如根路径不存在，`discover` 直接报 `STK-INP-0086` 返回，
                // 于是 `BatchProduced` 里一行 `Row` 都没有。
                //
                // 幸而 `finish()`（上面那两处调用）把没跑到的 `Queued` 行
                // 都转成了 `NotAttempted` —— 那也是可重试的，
                // 所以抽屉里**有行可重**。这条注释的目的是把那个依赖写明：
                // 如果哪天 `finish()` 不再做那个转换，这个死角会重新出现，
                // 而 `retrying_only_reruns_what_failed_and_really_produces_it`
                // 会当场报出来。
            }
            Message::TaskNotes(notes) => {
                for n in notes {
                    if !self.task_drawer.notes.contains(&n) {
                        self.task_drawer.notes.push(n);
                    }
                }
            }
            Message::CancelQueue => {
                tracing::info!("用户请求取消批处理");
                // 真的把令牌置位：worker 在每个 (输入, profile) 之前检查它。
                // 只改 is_running 而不置位，就是“按钮能点但没用”。
                self.cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                self.task_drawer.is_running = false;
                self.task_drawer
                    .notes
                    .push("已请求取消：正在跑的这一步会先做完，后续不再开始".to_string());
            }
            Message::ToggleTaskDrawer => self.task_drawer.toggle_expand(),
            Message::SelectOutputDir => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.task_drawer.output_dir = dir;
                }
            }
            Message::OpenOutputDir => {
                let dir = self.task_drawer.output_dir.clone();
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            // === 顶部栏 ===
            Message::OpenDoctorDialog => {
                self.show_doctor = true;
                // 真的跑一次 doctor（与 `subtitletool-cli doctor --json` 同一份实现）；
                // 放进阻塞线程因为要探 ffprobe/render-host/字体环境。
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(crate::worker::doctor_worker::run)
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r)
                    },
                    |r| Message::DoctorLoaded(Box::new(r)),
                );
            }
            Message::CloseDialog => self.show_doctor = false,
            Message::DoctorLoaded(report) => {
                self.doctor = Some(*report);
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let total_plan =
            self.inputs.selected_inputs().len() * self.profiles.enabled_profiles().len();

        let top_header = header::view();

        let main_workspace = row![
            inputs_view::view(&self.inputs),
            preview_view::view(&self.preview),
            profiles_view::view(&self.profiles),
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        let bottom_drawer = task_view::view(&self.task_drawer, total_plan);

        let mut full_layout = column![top_header, main_workspace, bottom_drawer,]
            .width(Length::Fill)
            .height(Length::Fill);

        // 自检弹窗：真有用内容才弹。以前 `show_doctor` 被设了却从不渲染，
        // 点了按钮什么都不会发生——用户只会以为自己点错了。
        if self.show_doctor {
            full_layout = full_layout.push(doctor_view(self.doctor.as_ref()));
        }

        container(full_layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn theme(&self) -> Theme {
        Theme::Light
    }
}

/// 环境自检面板。三态各有各的文案：没跑过 / 拿到了 / 拿不到。
fn doctor_view(report: Option<&Result<serde_json::Value, String>>) -> iced::Element<'_, Message> {
    use iced::widget::{button, column, container, row, text, Space};
    use iced::{Alignment, Length};

    let mut body = column![].spacing(4);
    match report {
        None => {
            body = body.push(text("正在运行环境自检…").size(12));
        }
        Some(Err(msg)) => {
            // 拿不到自检结果本身就是要报告的事，不是“弹窗内容为空”
            body = body.push(text(format!("自检未能完成：{msg}")).size(12));
        }
        Some(Ok(data)) => {
            for (k, v) in crate::worker::doctor_worker::rows(data) {
                body = body.push(
                    row![
                        text(k).size(12).width(Length::Fixed(120.0)),
                        text(v).size(12),
                    ]
                    .spacing(8),
                );
            }
        }
    }

    let close = button(text("关闭").size(12)).on_press(Message::CloseDialog);
    let panel = column![
        text("环境自检").size(14),
        Space::with_height(Length::Fixed(6.0)),
        body,
        Space::with_height(Length::Fixed(8.0)),
        row![close].align_y(Alignment::Center),
    ]
    .spacing(4);

    container(panel)
        .width(Length::Fill)
        .padding([10, 16])
        .into()
}
