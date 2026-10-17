use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum TaskStatus {
    Queued,
    Running {
        progress: f32,
        message: String,
    },
    Completed,
    Failed {
        code: String,
        reason: String,
        action: String,
    },
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct TaskItem {
    pub id: usize,
    pub input_name: String,
    pub profile_name: String,
    pub output_path: PathBuf,
    pub status: TaskStatus,
}

#[derive(Clone, Debug)]
pub struct TaskDrawerState {
    pub is_expanded: bool,
    pub output_dir: PathBuf,
    pub tasks: Vec<TaskItem>,
    pub is_running: bool,
}

impl Default for TaskDrawerState {
    fn default() -> Self {
        let default_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("outputs");
        Self {
            is_expanded: false,
            output_dir: default_dir,
            tasks: Vec::new(),
            is_running: false,
        }
    }
}

impl TaskDrawerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_expand(&mut self) {
        self.is_expanded = !self.is_expanded;
    }

    pub fn total_tasks(&self) -> usize {
        self.tasks.len()
    }

    pub fn completed_tasks(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .count()
    }

    pub fn failed_tasks(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .count()
    }
}
