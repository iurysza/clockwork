use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Public managed-job identity. Validated with the same safe-name rule the
/// managed source adapter has always enforced: directory name and job name
/// must match and contain only `[A-Za-z0-9._-]`, starting with an
/// alphanumeric, at most 64 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobName(String);

impl JobName {
    pub fn parse(input: &str) -> Result<Self, String> {
        if input.is_empty() || input.len() > 64 {
            return Err(format!(
                "invalid job name '{input}': must be 1-64 characters"
            ));
        }
        let mut chars = input.chars();
        let first = chars.next().unwrap_or(' ');
        if !first.is_ascii_alphanumeric() {
            return Err(format!(
                "invalid job name '{input}': must start with a letter or digit"
            ));
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return Err(format!(
                "invalid job name '{input}': only letters, digits, '.', '_' and '-' are allowed"
            ));
        }
        Ok(Self(input.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for JobName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for JobName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_names() {
        assert!(JobName::parse("daily-brief").is_ok());
        assert!(JobName::parse("a.1_x-9").is_ok());
        assert!(JobName::parse(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn rejects_unsafe_names() {
        assert!(JobName::parse("").is_err());
        assert!(JobName::parse("-lead").is_err());
        assert!(JobName::parse("has space").is_err());
        assert!(JobName::parse("../escape").is_err());
        assert!(JobName::parse(&"a".repeat(65)).is_err());
    }
}
