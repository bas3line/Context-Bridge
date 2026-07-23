//! Deterministic normalization, reduction, compaction, and handoff rendering.

mod budget;
mod compactor;
mod deterministic_summary;
mod handoff_builder;
mod normalizer;
mod reducer;
mod relevance;
pub mod renderers;

pub use budget::*;
pub use compactor::*;
pub use handoff_builder::*;
pub use normalizer::*;

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("context budget must be greater than zero")]
    InvalidBudget,
    #[error(
        "context budget {budget} is too small for required objectives and constraints; \
         at least approximately {required} tokens are needed"
    )]
    BudgetTooSmall { budget: usize, required: usize },
    #[error("could not serialize handoff while applying the context budget")]
    Serialize(#[from] serde_json::Error),
}
