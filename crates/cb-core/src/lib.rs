//! Vendor-neutral domain types and orchestration ports for Context Bridge.

pub mod domain;
pub mod ports;
pub mod services;

pub use domain::*;
pub use ports::*;
pub use services::*;
