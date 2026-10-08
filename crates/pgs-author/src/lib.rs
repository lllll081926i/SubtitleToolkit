//! PGS authoring 规划层（07 §组件、13 §4/§5）。
//!
//! `plan` 负责"哪些采样该合成一个显示集、哪些是 erase、对象在哪"，
//! 编码交给 `codec-pgs` Basic Encoder，自检交给 `validator`。
//! 三层各自可独立审：本层不写段格式，也不改校验标准。

pub mod plan;

pub use plan::author;
pub use plan::author_from_plan;
pub use plan::empty_digest;
pub use plan::plan;
pub use plan::AuthorOptions;
pub use plan::Authored;
pub use plan::Digest;
pub use plan::Event;
pub use plan::EventKind;
pub use plan::Plan;
pub use plan::PlanBuilder;
pub use plan::PlanStats;
pub use plan::Sample;
