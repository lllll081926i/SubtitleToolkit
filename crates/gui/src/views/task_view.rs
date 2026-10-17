use crate::icons;
use crate::message::Message;
use crate::state::task::{TaskDrawerState, TaskStatus};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

pub fn view<'a>(state: &'a TaskDrawerState, total_plan: usize) -> Element<'a, Message> {
    let action_btn = if state.is_running {
        button(
            row![icons::stop(14.0), text("取消正在执行的任务").size(13)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .on_press(Message::CancelQueue)
        .padding([8, 16])
    } else {
        button(
            row![
                icons::play(14.0),
                text(format!("开始执行队列 (共 {} 个任务)", total_plan)).size(13)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::StartQueue)
        .padding([8, 16])
    };

    let dir_text = format!("输出目录: {}", state.output_dir.display());

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
        Space::with_width(Length::Fixed(16.0)),
        dir_controls,
        Space::with_width(Length::Fill),
        text(format!(
            "已完成: {} / 失败: {}",
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
        if state.tasks.is_empty() {
            task_list = task_list
                .push(text("暂无任务历史，配置左侧输入与右侧Profile后点击开始。").size(12));
        } else {
            for task in &state.tasks {
                let status_row: Element<'a, Message> = match &task.status {
                    TaskStatus::Queued => text("排队中").size(11).into(),
                    TaskStatus::Running { progress, message } => {
                        text(format!("运行中 ({:.0}%): {}", progress * 100.0, message))
                            .size(11)
                            .into()
                    }
                    TaskStatus::Completed => {
                        row![icons::check(12.0), text("完成 (已通过原子校验)").size(11)]
                            .spacing(4)
                            .align_y(Alignment::Center)
                            .into()
                    }
                    TaskStatus::Failed {
                        code,
                        reason,
                        action,
                    } => row![
                        icons::close(12.0),
                        text(format!("失败 [{}] {}: {}", code, reason, action)).size(11)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .into(),
                    TaskStatus::Cancelled => text("已取消").size(11).into(),
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
