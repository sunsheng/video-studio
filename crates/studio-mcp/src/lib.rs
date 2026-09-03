//! video-studio 的 MCP 协议层。

pub mod server;
pub mod tools;
pub mod trace;

pub use server::Server;
pub use tools::{tool_list, tool_names, ToolSpec, TOOLS};
pub use trace::{Trace, TraceRecord};
