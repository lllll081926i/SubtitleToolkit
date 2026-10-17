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
}

impl Default for SubtitleApp {
    fn default() -> Self {
        Self {
            inputs: InputsState::new(),
            preview: PreviewState::new(),
            profiles: ProfilesState::new(),
            task_drawer: TaskDrawerState::new(),
            show_doctor: false,
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

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // === 左栏输入事件 ===
            Message::InputSelected(id) => {
                self.inputs.active_preview_id = Some(id);
                if let Some(item) = self.inputs.active_input() {
                    let path = item.path.clone();
                    let time = self.preview.current_time_ms;
                    self.preview.is_rendering = true;
                    return Task::perform(
                        async move {
                            crate::worker::preview_worker::render_single_frame(
                                path,
                                time,
                                (1920, 1080),
                            )
                            .await
                        },
                        |res| match res {
                            Ok((rgba, w, h)) => Message::FrameRendered {
                                rgba,
                                width: w,
                                height: h,
                            },
                            Err(_) => Message::FrameRendered {
                                rgba: vec![],
                                width: 1920,
                                height: 1080,
                            },
                        },
                    );
                }
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
                self.inputs.banner_suggestion = Some(format!(
                    "参考视频已载入 ({}): 建议帧率 23.976 | 分辨率 3840x2160",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ));
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
                if let Some(item) = self.inputs.active_input() {
                    let path = item.path.clone();
                    self.preview.is_rendering = true;
                    return Task::perform(
                        async move {
                            crate::worker::preview_worker::render_single_frame(
                                path,
                                time,
                                (1920, 1080),
                            )
                            .await
                        },
                        |res| match res {
                            Ok((rgba, w, h)) => Message::FrameRendered {
                                rgba,
                                width: w,
                                height: h,
                            },
                            Err(_) => Message::FrameRendered {
                                rgba: vec![],
                                width: 1920,
                                height: 1080,
                            },
                        },
                    );
                }
            }
            Message::PreviewJumpEvent(next) => {
                let step = if next { 2000 } else { -2000 };
                let t = (self.preview.current_time_ms + step).max(0);
                return self.update(Message::PreviewTimeChanged(t));
            }
            Message::ToggleSplitAb => self.preview.toggle_split_ab(),
            Message::FrameRendered {
                rgba,
                width,
                height,
            } => {
                if !rgba.is_empty() {
                    self.preview.set_frame(rgba, width, height);
                } else {
                    self.preview.is_rendering = false;
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
                let inputs: Vec<_> = self
                    .inputs
                    .selected_inputs()
                    .into_iter()
                    .map(|i| i.path.clone())
                    .collect();
                let profiles: Vec<_> = self
                    .profiles
                    .enabled_profiles()
                    .into_iter()
                    .cloned()
                    .collect();
                if inputs.is_empty() || profiles.is_empty() {
                    return Task::none();
                }
                self.task_drawer.is_running = true;
                self.task_drawer.tasks.clear();

                for inp in &inputs {
                    for prof in &profiles {
                        let id = self.task_drawer.tasks.len() + 1;
                        self.task_drawer.tasks.push(crate::state::task::TaskItem {
                            id,
                            input_name: inp
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into(),
                            profile_name: prof.name.clone(),
                            output_path: self
                                .task_drawer
                                .output_dir
                                .join(format!("{}.out", prof.name)),
                            status: crate::state::task::TaskStatus::Queued,
                        });
                    }
                }
            }
            Message::CancelQueue => {
                self.task_drawer.is_running = false;
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
            Message::TaskProgress {
                id,
                progress,
                message,
            } => {
                if let Some(t) = self.task_drawer.tasks.iter_mut().find(|t| t.id == id) {
                    t.status = crate::state::task::TaskStatus::Running { progress, message };
                }
            }
            Message::TaskFinished { id, result } => {
                if let Some(t) = self.task_drawer.tasks.iter_mut().find(|t| t.id == id) {
                    match result {
                        Ok(()) => t.status = crate::state::task::TaskStatus::Completed,
                        Err((code, reason, action)) => {
                            t.status = crate::state::task::TaskStatus::Failed {
                                code,
                                reason,
                                action,
                            }
                        }
                    }
                }
            }

            // === 顶部栏 ===
            Message::OpenDoctorDialog => self.show_doctor = true,
            Message::CloseDialog => self.show_doctor = false,
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

        let full_layout = column![top_header, main_workspace, bottom_drawer,]
            .width(Length::Fill)
            .height(Length::Fill);

        container(full_layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }
}
