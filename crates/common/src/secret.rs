use std::fmt;

/// Environment variable holding the Riot API key.
pub const RIOT_KEY_ENV: &str = "RIOT_API_KEY";

/// A secret string that can never be printed, logged, or serialized by accident.
/// `Debug` and `Display` both render `[REDACTED]`; there is deliberately no
/// `Serialize` impl. Use [`Secret::expose`] only at the point of use (e.g. an
/// HTTP header marked sensitive).
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Where a key was found. Safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    ProcessEnv,
    UserEnv,
}

/// Looks up the Riot API key: process environment first, then (on Windows) the
/// user-scope environment in the registry, so a key set after this process's
/// parent shell started is still found. Credential Manager storage arrives with
/// the first-run wizard.
pub fn load_riot_api_key() -> Option<(Secret, KeySource)> {
    if let Some(key) = std::env::var(RIOT_KEY_ENV).ok().and_then(non_empty) {
        return Some((Secret::new(key), KeySource::ProcessEnv));
    }
    user_env_value(RIOT_KEY_ENV).and_then(non_empty).map(|key| (Secret::new(key), KeySource::UserEnv))
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(windows)]
fn user_env_value(name: &str) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment").ok()?.get_value::<String, _>(name).ok()
}

#[cfg(not(windows))]
fn user_env_value(_name: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_never_formats_its_value() {
        let s = Secret::new("RGAPI-super-secret");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert_eq!(format!("{s:?}"), "[REDACTED]");
        assert_eq!(format!("{:?}", Some(s.clone())), "Some([REDACTED])");
        assert_eq!(s.expose(), "RGAPI-super-secret");
    }

    #[test]
    fn non_empty_trims() {
        assert_eq!(non_empty("  abc \n".into()).as_deref(), Some("abc"));
        assert_eq!(non_empty("   ".into()), None);
    }
}
