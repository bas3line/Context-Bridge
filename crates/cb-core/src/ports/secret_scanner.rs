use crate::Sensitivity;

pub trait SecretScanner: Send + Sync {
    fn classify(&self, text: &str) -> Sensitivity;
    fn redact(&self, text: &str) -> String;
}
