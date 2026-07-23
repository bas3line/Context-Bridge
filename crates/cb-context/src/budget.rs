pub trait TokenEstimator: Send + Sync {
    fn estimate(&self, text: &str) -> usize;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ApproximateTokenEstimator;

impl TokenEstimator for ApproximateTokenEstimator {
    fn estimate(&self, text: &str) -> usize {
        text.chars().count().div_ceil(4)
    }
}
