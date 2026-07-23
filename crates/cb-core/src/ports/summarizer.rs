use crate::{ContextEvent, HandoffPackage};

pub trait Summarizer: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn summarize(
        &self,
        events: &[ContextEvent],
        package: &mut HandoffPackage,
    ) -> Result<(), Self::Error>;
}
