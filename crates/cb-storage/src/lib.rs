//! Crash-safe SQLite persistence for canonical Context Bridge state.

mod artifact_repository;
mod event_repository;
mod migrations;
mod session_repository;
mod sqlite;
mod transaction;

pub use sqlite::{SqliteStore, StorageError};
