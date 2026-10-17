//! Request/response shapes for each provider, against a mock server.

use rc_ai::{AiClient, AiError, CompletionRequest, ProviderConfig, ProviderKind};
use rc_common::Secret;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req() -> CompletionRequest {
    CompletionRequest { system: "sys".into(), user: "hello".into(), max_tokens: 100, json: true }
}

fn client(kind: ProviderKind, server: &MockServer, key: Option<&str>) -> AiClient {
    let base = match kind {
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => format!("{}/v1", server.uri()),
        _ => server.uri(),
    };
    AiClient::new(ProviderConfig { kind, model: "test-model".into(), base_url: Some(base) }, key.map(Secret::new))
        .expect("client")
}

#[tokio::test]
async fn anthropic_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "k1"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({"model": "test-model", "system": "sys", "max_tokens": 100})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "{\"ok\":true}"}],
            "usage": {"input_tokens": 12, "output_tokens": 5}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(ProviderKind::Anthropic, &server, Some("k1")).complete(&req()).await.expect("ok");
    assert_eq!(c.text, "{\"ok\":true}");
    assert_eq!((c.input_tokens, c.output_tokens), (Some(12), Some(5)));
}

#[tokio::test]
async fn openai_chat_with_json_mode() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer k2"))
        .and(body_partial_json(json!({"response_format": {"type": "json_object"}, "max_completion_tokens": 100})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "{}"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(ProviderKind::OpenAi, &server, Some("k2")).complete(&req()).await.expect("ok");
    assert_eq!(c.text, "{}");
}

#[tokio::test]
async fn openai_compatible_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"max_tokens": 100})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices": [{"message": {"content": "hi"}}]})))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(ProviderKind::OpenAiCompatible, &server, None).complete(&req()).await.expect("ok");
    assert_eq!(c.text, "hi");
}

#[tokio::test]
async fn gemini_generate_content() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/test-model:generateContent"))
        .and(header("x-goog-api-key", "k3"))
        .and(body_partial_json(json!({"generationConfig": {"responseMimeType": "application/json"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"parts": [{"text": "{\"a\":"}, {"text": "1}"}]}}],
            "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 2}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(ProviderKind::Gemini, &server, Some("k3")).complete(&req()).await.expect("ok");
    assert_eq!(c.text, "{\"a\":1}");
}

#[tokio::test]
async fn errors_are_classified_and_scrubbed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "invalid x-api-key: sk-leak"}})),
        )
        .mount(&server)
        .await;
    let err = client(ProviderKind::Anthropic, &server, Some("sk-leak")).complete(&req()).await.expect_err("401");
    match err {
        AiError::Unauthorized(msg) => {
            assert!(!msg.contains("sk-leak"), "key must be scrubbed: {msg}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn lists_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "models/gemini-a", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/embed", "supportedGenerationMethods": ["embedContent"]}
        ]})))
        .mount(&server)
        .await;
    let models = client(ProviderKind::Gemini, &server, Some("k")).list_models().await.expect("ok");
    assert_eq!(models, vec!["gemini-a".to_string()]);
}
