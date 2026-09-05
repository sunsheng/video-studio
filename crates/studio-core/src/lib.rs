//! video-studio 领域层。
//!
//! 这个 crate **不做任何 I/O**：没有文件、没有网络、没有数据库。
//! 阶段图、状态机、错误契约和 schema 校验都能在一台没有 GPU、
//! 没有 ComfyUI、没有 ffmpeg 的机器上完整地单元测试。
//!
//! 上层 crate 负责把这里的决策落到 SQLite 与文件系统。

pub mod assembly;
pub mod capability;
pub mod contract;
pub mod error;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod lexicon;
pub mod quality;
pub mod rubric;
pub mod schema;
pub mod stage;
pub mod state;

pub use assembly::{Fragment, FragmentKind, FragmentSet, ShotDeclaration, ShotSegment};
pub use capability::{CapabilitySet, WorkflowCapability, INJECTABLE_PARAMS};
pub use contract::{
    ActionKind, AnswerOption, Blocked, Confirmation, Decision, DecisionKind, Envelope, Event,
    NextAction, Outcome, Progress, ProjectInfo, ProjectStatus, Question, SelectionType, WaitingOn,
};
pub use error::{Result, StudioError, Violation};
pub use quality::{Finding, Metric, Severity};
pub use rubric::{RubricItem, SelfReview};
pub use stage::{Capability, StageId, StageKind, StageSpec, STAGE_GRAPH};
pub use state::{Approved, AwaitingConfirmation, Draft, LoadedStage, Stage, StageState, Submitted};

/// 阶段产物。形状由 [`schema`] 约束，内容由 Agent（creative）或控制面（deterministic）给出。
pub type Outputs = serde_json::Map<String, serde_json::Value>;
