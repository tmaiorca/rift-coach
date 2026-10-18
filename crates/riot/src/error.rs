/// Errors from the Riot client. Messages never contain the API key (it is sent
/// only as a sensitive header, never in URLs).
#[derive(Debug, thiserror::Error)]
pub enum RiotError {
    #[error("no Riot API key configured")]
    KeyMissing,
    /// 401/403: key invalid, expired, revoked, or not allowed this endpoint.
    #[error("Riot API key rejected (HTTP {status})")]
    KeyRejected { status: u16 },
    #[error("not found")]
    NotFound,
    #[error("bad request (HTTP {status}): {body}")]
    BadRequest { status: u16, body: String },
    #[error("Riot server error (HTTP {status}) after retries")]
    Server { status: u16 },
    #[error("still rate limited after {attempts} retries")]
    RateLimited { attempts: u32 },
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("could not decode response: {0}")]
    Decode(#[from] serde_json::Error),
    /// A call made with arguments the API can't take.
    #[error("{0}")]
    Invalid(String),
}

impl RiotError {
    /// Worth retrying later without changing anything.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Server { .. } | Self::RateLimited { .. } | Self::Http(_))
    }

    /// Requires user action on the key before any further call can succeed.
    pub fn is_key_problem(&self) -> bool {
        matches!(self, Self::KeyMissing | Self::KeyRejected { .. })
    }
}
