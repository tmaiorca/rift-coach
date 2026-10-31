//! LLM providers behind one interface. Keys arrive as [`Secret`] and are only
//! ever placed in a sensitive request header; error text is scrubbed of them.
//! No inference happens locally, and nothing here decides *when* AI may run
//! (the coach only calls it pre-game or post-game, per Riot policy).

use std::time::Duration;

use base64::Engine as _;
use rc_common::Secret;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(180);
/// Requests with images upload a few MB and take longer to answer.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(360);
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Gemini,
    /// Any server speaking the OpenAI chat-completions API (Ollama, LM Studio,
    /// vLLM, OpenRouter, a model on another PC on the LAN, ...).
    OpenAiCompatible,
}

impl ProviderKind {
    pub const ALL: [Self; 4] = [Self::Anthropic, Self::OpenAi, Self::Gemini, Self::OpenAiCompatible];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "open_ai",
            Self::Gemini => "gemini",
            Self::OpenAiCompatible => "open_ai_compatible",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic (Claude)",
            Self::OpenAi => "OpenAI",
            Self::Gemini => "Google Gemini",
            Self::OpenAiCompatible => "OpenAI-compatible (custom URL)",
        }
    }

    /// Windows Credential Manager entry holding this provider's key.
    pub fn credential_name(self) -> String {
        format!("ai_{}", self.as_str())
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com",
            Self::OpenAiCompatible => "http://localhost:11434/v1",
        }
    }

    /// Suggested model when none is configured (the user can pick any model
    /// the provider lists).
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-5-5",
            _ => "",
        }
    }

    /// Local/LAN servers often need no key.
    pub fn key_required(self) -> bool {
        !matches!(self, Self::OpenAiCompatible)
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|k| k.as_str() == s).ok_or_else(|| format!("unknown AI provider '{s}'"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub model: String,
    /// Overrides the provider's default endpoint.
    #[serde(default)]
    pub base_url: Option<String>,
}

impl ProviderConfig {
    pub fn base_url(&self) -> String {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(self.kind.default_base_url())
            .trim_end_matches('/')
            .to_owned()
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    /// Ask the provider for JSON output where it supports a JSON mode.
    pub json: bool,
    /// Images sent after `user`, each preceded by its caption. Empty for
    /// text-only requests (the request is then sent exactly as before).
    pub images: Vec<ImagePart>,
}

/// One image for a vision request.
#[derive(Clone)]
pub struct ImagePart {
    /// Text placed right before the image (e.g. which moment and time it shows).
    pub caption: String,
    /// `image/jpeg` or `image/png`.
    pub media_type: &'static str,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for ImagePart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagePart")
            .field("caption", &self.caption)
            .field("media_type", &self.media_type)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl ImagePart {
    fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("no API key saved for {0}")]
    MissingKey(&'static str),
    #[error("no model selected")]
    MissingModel,
    #[error("the provider rejected the API key ({0})")]
    Unauthorized(String),
    #[error("the provider is rate limiting or out of quota ({0})")]
    RateLimited(String),
    #[error("provider returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not reach the provider: {0}")]
    Network(String),
    #[error("unexpected provider response: {0}")]
    BadResponse(String),
}

impl AiError {
    /// Worth retrying later without changing anything.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::RateLimited(_) | Self::Network(_) => true,
            Self::Http { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

pub struct AiClient {
    http: reqwest::Client,
    config: ProviderConfig,
    key: Option<Secret>,
}

impl std::fmt::Debug for AiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiClient").field("config", &self.config).field("key", &self.key).finish()
    }
}

impl AiClient {
    pub fn new(config: ProviderConfig, key: Option<Secret>) -> Result<Self, AiError> {
        if key.is_none() && config.kind.key_required() {
            return Err(AiError::MissingKey(config.kind.label()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("RiftCoach/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| AiError::Network(e.to_string()))?;
        Ok(Self { http, config, key })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn headers(&self) -> Result<HeaderMap, AiError> {
        let mut h = HeaderMap::new();
        let Some(key) = &self.key else { return Ok(h) };
        let (name, value) = match self.config.kind {
            ProviderKind::Anthropic => ("x-api-key", key.expose().to_owned()),
            ProviderKind::Gemini => ("x-goog-api-key", key.expose().to_owned()),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                ("authorization", format!("Bearer {}", key.expose()))
            }
        };
        let mut v =
            HeaderValue::from_str(&value).map_err(|_| AiError::BadResponse("API key has invalid characters".into()))?;
        v.set_sensitive(true);
        h.insert(name, v);
        if self.config.kind == ProviderKind::Anthropic {
            h.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));
        }
        Ok(h)
    }

    /// Removes the key from provider error text, in case one echoes it.
    fn scrub(&self, text: &str) -> String {
        let mut s: String = text.chars().take(400).collect();
        if let Some(k) = &self.key
            && !k.expose().is_empty()
        {
            s = s.replace(k.expose(), "[REDACTED]");
        }
        s
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, AiError> {
        let resp =
            req.headers(self.headers()?).send().await.map_err(|e| AiError::Network(self.scrub(&e.to_string())))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(|e| AiError::Network(self.scrub(&e.to_string())))?;
        if !(200..300).contains(&status) {
            let message = self.scrub(&error_message(&body));
            return Err(match status {
                401 | 403 => AiError::Unauthorized(message),
                429 => AiError::RateLimited(message),
                _ => AiError::Http { status, message },
            });
        }
        serde_json::from_str(&body).map_err(|e| AiError::BadResponse(format!("not JSON: {e}")))
    }

    pub async fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError> {
        let model = self.config.model.trim();
        if model.is_empty() {
            return Err(AiError::MissingModel);
        }
        let base = self.config.base_url();
        tracing::info!(provider = self.config.kind.as_str(), model, images = req.images.len(), "AI request");
        let timeout = if req.images.is_empty() { TIMEOUT } else { IMAGE_TIMEOUT };
        let post = |url: String, body: &Value| self.http.post(url).timeout(timeout).json(body);
        match self.config.kind {
            ProviderKind::Anthropic => {
                let content = if req.images.is_empty() {
                    json!(req.user)
                } else {
                    let mut parts = vec![json!({"type": "text", "text": req.user})];
                    for img in &req.images {
                        parts.push(json!({"type": "text", "text": img.caption}));
                        parts.push(json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": img.media_type, "data": img.base64()},
                        }));
                    }
                    Value::Array(parts)
                };
                let body = json!({
                    "model": model,
                    "max_tokens": req.max_tokens,
                    "system": req.system,
                    "messages": [{"role": "user", "content": content}],
                });
                let v = self.send(post(format!("{base}/v1/messages"), &body)).await?;
                let text = v["content"]
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter(|p| p["type"] == "text")
                            .filter_map(|p| p["text"].as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                Ok(Completion {
                    text: non_empty(text)?,
                    input_tokens: v["usage"]["input_tokens"].as_i64(),
                    output_tokens: v["usage"]["output_tokens"].as_i64(),
                })
            }
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                let content = if req.images.is_empty() {
                    json!(req.user)
                } else {
                    let mut parts = vec![json!({"type": "text", "text": req.user})];
                    for img in &req.images {
                        parts.push(json!({"type": "text", "text": img.caption}));
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", img.media_type, img.base64()),
                                "detail": "high",
                            },
                        }));
                    }
                    Value::Array(parts)
                };
                let mut body = json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": req.system},
                        {"role": "user", "content": content},
                    ],
                });
                if self.config.kind == ProviderKind::OpenAi {
                    body["max_completion_tokens"] = json!(req.max_tokens);
                    if req.json {
                        body["response_format"] = json!({"type": "json_object"});
                    }
                } else {
                    body["max_tokens"] = json!(req.max_tokens);
                }
                let v = self.send(post(format!("{base}/chat/completions"), &body)).await?;
                let text = v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_owned();
                Ok(Completion {
                    text: non_empty(text)?,
                    input_tokens: v["usage"]["prompt_tokens"].as_i64(),
                    output_tokens: v["usage"]["completion_tokens"].as_i64(),
                })
            }
            ProviderKind::Gemini => {
                let mut config = json!({"maxOutputTokens": req.max_tokens});
                if req.json {
                    config["responseMimeType"] = json!("application/json");
                }
                let mut parts = vec![json!({"text": req.user})];
                for img in &req.images {
                    parts.push(json!({"text": img.caption}));
                    parts.push(json!({"inline_data": {"mime_type": img.media_type, "data": img.base64()}}));
                }
                let body = json!({
                    "systemInstruction": {"parts": [{"text": req.system}]},
                    "contents": [{"role": "user", "parts": parts}],
                    "generationConfig": config,
                });
                let model = model.trim_start_matches("models/");
                let v = self.send(post(format!("{base}/v1beta/models/{model}:generateContent"), &body)).await?;
                let text = v["candidates"][0]["content"]["parts"]
                    .as_array()
                    .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect::<String>())
                    .unwrap_or_default();
                Ok(Completion {
                    text: non_empty(text)?,
                    input_tokens: v["usageMetadata"]["promptTokenCount"].as_i64(),
                    output_tokens: v["usageMetadata"]["candidatesTokenCount"].as_i64(),
                })
            }
        }
    }

    /// Models the provider offers to this key, for the settings picker.
    pub async fn list_models(&self) -> Result<Vec<String>, AiError> {
        let base = self.config.base_url();
        let mut models: Vec<String> = match self.config.kind {
            ProviderKind::Anthropic => {
                let v = self.send(self.http.get(format!("{base}/v1/models?limit=100"))).await?;
                ids(&v["data"], "id")
            }
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                let v = self.send(self.http.get(format!("{base}/models"))).await?;
                ids(&v["data"], "id")
            }
            ProviderKind::Gemini => {
                let v = self.send(self.http.get(format!("{base}/v1beta/models?pageSize=200"))).await?;
                v["models"]
                    .as_array()
                    .map(|ms| {
                        ms.iter()
                            .filter(|m| {
                                m["supportedGenerationMethods"]
                                    .as_array()
                                    .is_some_and(|a| a.iter().any(|x| x == "generateContent"))
                            })
                            .filter_map(|m| m["name"].as_str())
                            .map(|n| n.trim_start_matches("models/").to_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        models.sort();
        models.dedup();
        Ok(models)
    }
}

fn ids(v: &Value, field: &str) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|m| m[field].as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn non_empty(text: String) -> Result<String, AiError> {
    if text.trim().is_empty() { Err(AiError::BadResponse("empty completion".into())) } else { Ok(text) }
}

/// Pulls a readable message out of a provider error body.
fn error_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    parsed
        .as_ref()
        .and_then(|v| v["error"]["message"].as_str().or_else(|| v["error"].as_str()).or_else(|| v["message"].as_str()))
        .map_or_else(|| body.trim().to_owned(), str::to_owned)
}

/// Extracts the JSON object from a completion that may wrap it in prose or a
/// ```json fence.
pub fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| serde_json::from_str(&text[start..=end]).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fenced_json() {
        let v = extract_json("Here you go:\n```json\n{\"a\": 1}\n```").expect("json");
        assert_eq!(v["a"], 1);
        assert!(extract_json("no json here").is_none());
    }

    #[test]
    fn debug_never_shows_key() {
        let c = AiClient::new(
            ProviderConfig { kind: ProviderKind::Anthropic, model: "m".into(), base_url: None },
            Some(Secret::new("sk-ant-secret")),
        )
        .expect("client");
        assert!(!format!("{c:?}").contains("sk-ant-secret"));
        assert_eq!(c.scrub("bad key sk-ant-secret"), "bad key [REDACTED]");
    }

    #[test]
    fn key_requirements() {
        let cfg = |kind| ProviderConfig { kind, model: "m".into(), base_url: None };
        assert!(matches!(AiClient::new(cfg(ProviderKind::OpenAi), None), Err(AiError::MissingKey(_))));
        assert!(AiClient::new(cfg(ProviderKind::OpenAiCompatible), None).is_ok());
    }
}
