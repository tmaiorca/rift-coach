//! Builds the configured AI client: settings say which provider and model,
//! Windows Credential Manager holds the key.

use rc_ai::{AiClient, ProviderConfig, ProviderKind};
use rc_common::credentials;
use rc_store::settings::{AiSettings, VisionSettings};

use crate::CoachError;

pub fn provider_config(s: &AiSettings) -> Result<ProviderConfig, CoachError> {
    let kind: ProviderKind = s.provider.parse().map_err(CoachError::Invalid)?;
    let model = if s.model.trim().is_empty() { kind.default_model().to_owned() } else { s.model.trim().to_owned() };
    Ok(ProviderConfig { kind, model, base_url: s.base_url.clone() })
}

pub fn client_from_settings(s: &AiSettings) -> Result<AiClient, CoachError> {
    let config = provider_config(s)?;
    let key =
        credentials::load_secret(&config.kind.credential_name()).map_err(|e| CoachError::Invalid(e.to_string()))?;
    Ok(AiClient::new(config, key)?)
}

/// The vision review's client: its own provider and model, the provider's
/// key from Credential Manager (shared with text reviews).
pub fn vision_client(s: &VisionSettings) -> Result<AiClient, CoachError> {
    client_from_settings(&AiSettings {
        provider: s.provider.clone(),
        model: s.model.clone(),
        base_url: s.base_url.clone(),
        ..AiSettings::default()
    })
}
