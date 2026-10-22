use crate::icons;
use crate::message::Message;
use crate::state::task::{TaskDrawerState, TaskStatus};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

pub fn view<'a>(state: &'a TaskDrawerState, total_plan: usize) -> Element<'a, Message> {
    let action_btn = if state.is_running {
        button(
            row![icons::stop(14.0), text("取消").size(13)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .on_press(Message::CancelQueue)
        .padding([8, 16])
    } else {
        button(
            row![
                icons::play(14.0),
                text(format!("开始处理（{} 项）", total_plan)).size(13)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::StartQueue)
        .padding([8, 16])
    };

    // AT-RENDER-002：失败之后必须能重试，而不只是“看得到红了”。
    //
    // 只重跑失败与没跑到的（见 `TaskDrawerState::retry_targets`）——
    // 不重写已经好的产物。没有可重试的格子时**按钮不出现**：
    // 一个点了没反应的按钮比没有按钮更差（用户会以为界面卡了）。
    let retry_count = state.retry_targets().len();
    let retry_btn: Option<Element<'a, Message>> = if !state.is_running && retry_count > 0 {
        Some(
            button(
                row![
                    icons::play(14.0),
                    text(format!("重试失败项（{retry_count}）")).size(13)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .on_press(Message::RetryFailed)
            .padding([8, 16])
            .into(),
        )
    } else {
        None
    };

    let dir_text = format!("输出: {}", state.output_dir.display());

    let dir_controls = row![
        text(dir_text).size(12),
        button(text("更改...").size(11))
            .on_press(Message::SelectOutputDir)
            .padding([2, 6]),
        button(
            row![icons::folder(12.0), text("打开文件夹").size(11)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .on_press(Message::OpenOutputDir)
        .padding([2, 6]),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let toggle_expand_btn = button(
        row![
            if state.is_expanded {
                icons::chevron_down(12.0)
            } else {
                icons::chevron_up(12.0)
            },
            text(if state.is_expanded {
                "收起"
            } else {
                "详情"
            })
            .size(11),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .on_press(Message::ToggleTaskDrawer)
    .padding([4, 8]);

    let bar_content = row![
        action_btn,
        // 重试按钮紧跟在开始/取消后面：它们是同一条动作链上的两步。
        // `Option` 在 `row!` 里不能直接展开，所以用一个零宽的空元素占位。
        match retry_btn {
            Some(b) => row![Space::with_width(Length::Fixed(8.0)), b],
            None => row![],
        },
        Space::with_width(Length::Fixed(16.0)),
        dir_controls,
        Space::with_width(Length::Fill),
        text(format!(
            "完成 {} · 失败 {}",
            state.completed_tasks(),
            state.failed_tasks()
        ))
        .size(12),
        Space::with_width(Length::Fixed(12.0)),
        toggle_expand_btn,
    ]
    .align_y(Alignment::Center);

    let mut main_col = column![bar_content].spacing(8);

    if state.is_expanded {
        let mut task_list = column![].spacing(4);

        // 编译期“做不到”的说明与报告的 warnings 先摆出来：
        // 它们解释了“为什么选了这个参数却没生效”，藏在抽屉最底下等于没说。
        if !state.notes.is_empty() {
            let mut notes_col = column![].spacing(2);
            for n in &state.notes {
                notes_col = notes_col.push(text(format!("· {n}")).size(11));
            }
            task_list = task_list.push(container(notes_col).padding(4).width(Length::Fill));
        }

        if let Some(outcome) = &state.outcome {
            task_list = task_list.push(
                text(format!(
                    "整体判定：{outcome}（逐行计算得 {}）",
                    state.derived_outcome()
                ))
                .size(12),
            );
        }
        if let Some(p) = &state.spec_path {
            task_list = task_list
                .push(text(format!("spec（可直接喂给 CLI 重放）：{}", p.display())).size(11));
        }
        if let Some(p) = &state.report_path {
            task_list = task_list.push(text(format!("报告：{}", p.display())).size(11));
        }

        if state.tasks.is_empty() {
            task_list = task_list.push(text("暂无任务").size(12));
        } else {
            for task in &state.tasks {
                // 状态文字直接取自 `TaskStatus::label()` 与 `detail()`：
                // 这里不再自己把状态“翻译”一遍，否则 GUI 与 CLI 报告会出现两套词。
                let status_row: Element<'a, Message> = match &task.status {
                    TaskStatus::Verified { .. } | TaskStatus::FromCache { .. } => {
                        row![icons::check(12.0), text(status_line(&task.status)).size(11)]
                            .spacing(4)
                            .align_y(Alignment::Center)
                            .into()
                    }
                    TaskStatus::WouldWrite { .. } => {
                        text(status_line(&task.status)).size(11).into()
                    }
                    TaskStatus::Failed { code, .. } => row![
                        icons::close(12.0),
                        text(format!("失败 [{code}] {}", task.status.detail())).size(11)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .into(),
                    TaskStatus::NotAttempted { .. } => {
                        text(status_line(&task.status)).size(11).into()
                    }
                    TaskStatus::Queued | TaskStatus::Running | TaskStatus::Cancelled { .. } => {
                        text(status_line(&task.status)).size(11).into()
                    }
                };

                let item_row = row![
                    text(&task.input_name).size(12).width(Length::Fixed(180.0)),
                    icons::arrow_right(12.0),
                    text(&task.profile_name)
                        .size(12)
                        .width(Length::Fixed(180.0)),
                    Space::with_width(Length::Fill),
                    status_row,
                ]
                .spacing(8)
                .align_y(Alignment::Center);

                task_list = task_list.push(container(item_row).padding(4).width(Length::Fill));
            }
        }
        main_col = main_col.push(scrollable(task_list).height(Length::Fixed(120.0)));
    }

    container(main_col)
        .width(Length::Fill)
        .padding([8, 16])
        .into()
}

/// 一行状态文字：`label + detail`。detail 为空时只给 label，不留个悬空的冒号。
fn status_line(s: &TaskStatus) -> String {
    let d = s.detail();
    if d.is_empty() {
        s.label().to_string()
    } else {
        format!("{}：{d}", s.label())
    }
}
