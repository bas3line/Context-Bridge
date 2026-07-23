//! The child inherits the caller's controlling PTY in Phase 1.
//!
//! This preserves colors, resize events, terminal modes, and foreground signal
//! delivery without interposing a lossy terminal parser. A newly allocated PTY
//! is reserved for callers that do not already own an interactive terminal.
