//! Capability-driven integrations for Codex CLI, Claude Code, and OpenCode.

pub mod claude;
pub mod codex;
mod documented;
pub mod opencode;
mod registry;

pub use documented::DocumentedCliAdapter;
pub use registry::{AdapterRegistry, CliAdapter};
