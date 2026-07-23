use crate::{ContextEvent, Sensitivity};

#[derive(Debug, Default)]
pub struct ExportService;

impl ExportService {
    #[must_use]
    pub fn visible_events(events: &[ContextEvent], redacted: bool) -> Vec<&ContextEvent> {
        events
            .iter()
            .filter(|event| event.sensitivity != Sensitivity::Excluded)
            .filter(|event| !redacted || event.sensitivity != Sensitivity::Secret)
            .collect()
    }
}
