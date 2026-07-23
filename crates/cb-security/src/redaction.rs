use cb_core::{SecretScanner, Sensitivity};

use crate::{ASSIGNMENT_MARKERS, POTENTIAL_SECRET_MARKERS, SECRET_MARKERS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionLevel {
    Off,
    Standard,
    Strict,
}

#[derive(Debug, Clone)]
pub struct LocalSecretScanner {
    level: RedactionLevel,
}

impl LocalSecretScanner {
    #[must_use]
    pub const fn new(level: RedactionLevel) -> Self {
        Self { level }
    }

    fn line_is_secret(line: &str) -> bool {
        let lower = line.to_ascii_lowercase();
        SECRET_MARKERS.iter().any(|marker| line.contains(marker))
            || ASSIGNMENT_MARKERS
                .iter()
                .any(|marker| lower.contains(marker))
    }

    fn line_is_potential_secret(&self, line: &str) -> bool {
        self.level == RedactionLevel::Strict
            && POTENTIAL_SECRET_MARKERS
                .iter()
                .any(|marker| line.to_ascii_lowercase().contains(marker))
    }

    fn line_should_redact(&self, line: &str) -> bool {
        Self::line_is_secret(line) || self.line_is_potential_secret(line)
    }
}

impl Default for LocalSecretScanner {
    fn default() -> Self {
        Self::new(RedactionLevel::Strict)
    }
}

impl SecretScanner for LocalSecretScanner {
    fn classify(&self, text: &str) -> Sensitivity {
        if self.level == RedactionLevel::Off {
            return Sensitivity::Normal;
        }
        if text.lines().any(Self::line_is_secret) {
            Sensitivity::Secret
        } else if self.level == RedactionLevel::Strict
            && POTENTIAL_SECRET_MARKERS
                .iter()
                .any(|marker| text.to_ascii_lowercase().contains(marker))
        {
            Sensitivity::PotentialSecret
        } else {
            Sensitivity::Normal
        }
    }

    fn redact(&self, text: &str) -> String {
        if self.level == RedactionLevel::Off {
            return text.to_owned();
        }
        text.lines()
            .map(|line| {
                if self.line_should_redact(line) {
                    "[REDACTED BY CONTEXT BRIDGE]"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use cb_core::{SecretScanner, Sensitivity};

    use super::{LocalSecretScanner, RedactionLevel};

    #[test]
    fn detects_and_redacts_assignment_secrets() {
        let scanner = LocalSecretScanner::new(RedactionLevel::Strict);
        let input = "safe=true\nAPI_KEY=very-secret-value\nstill-safe=true";
        assert_eq!(scanner.classify(input), Sensitivity::Secret);
        let output = scanner.redact(input);
        assert!(!output.contains("very-secret-value"));
        assert!(output.contains("safe=true"));
    }

    #[test]
    fn strict_mode_redacts_potential_secret_lines_fail_closed() {
        let scanner = LocalSecretScanner::new(RedactionLevel::Strict);
        let input = "Use access token: raw-token-value";
        assert_eq!(scanner.classify(input), Sensitivity::PotentialSecret);
        assert_eq!(scanner.redact(input), "[REDACTED BY CONTEXT BRIDGE]");
    }
}
