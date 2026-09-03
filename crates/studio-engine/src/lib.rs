//! video-studio 控制面。
//!
//! 这一层把 [`studio_core`] 的状态机决策落到磁盘：bundle 布局、进程锁、
//! SQLite 状态、人可读的阶段产物镜像。

pub mod bundle;
pub mod config;
pub mod executor;
pub mod project;

pub use bundle::{Bundle, LockGuard};
pub use config::Settings;
pub use executor::{
    ExecContext, ExecRecord, ExecRecorder, ProgressNote, SharedExecutor, StageExecutor,
};
pub use project::{init_project, ExportResult, Project};
