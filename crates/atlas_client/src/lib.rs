#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod acta;
pub mod custos;
pub mod helpers;
pub mod platform;

pub use acta::Acta;
pub use custos::Custos;
pub use platform::Platform;

use atlas_api::{
    dtos::{
        HealthResponse, LoginRequest, LoginResponse, documents::ConflictProblemDto,
        lifecycle::PurgeStatusDtoResponse,
    },
    problem::ProblemDetails,
};
use std::time::Duration;
use thiserror::Error;

/// Maximum number of times a request is retried after a 429 before giving up.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
/// Upper bound on how long a single retry waits, regardless of `Retry-After`.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
/// Floor applied to any retry wait so a `Retry-After: 0` still yields a pause.
const MIN_RETRY_WAIT: Duration = Duration::from_millis(50);
/// Total per-request timeout applied to every API call by default.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Time allowed to establish a TCP/TLS connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Relaxed total timeout for attachment uploads/downloads, whose bodies can be
/// large enough that the default request timeout would abort a healthy transfer.
const ATTACHMENT_TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);

/// Renders an RFC 9457 problem as a readable one-line message, preserving the
/// historical `API error {status}: {title}` prefix and appending `detail` and
/// `hint` when the server provided them.
fn api_error_message(problem: &ProblemDetails) -> String {
    let mut message = format!("API error {}: {}", problem.status, problem.title);

    if let Some(detail) = problem.detail.as_deref() {
        message.push_str(&format!(" — {detail}"));
    }
    if let Some(hint) = problem.hint.as_deref() {
        message.push_str(&format!(" (hint: {hint})"));
    }

    message
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{}", api_error_message(.0))]
    Api(ProblemDetails),
    /// CAS revision conflict (HTTP 409) carrying the head revision and the patch
    /// from the client's stale base to the current content, so callers can apply
    /// the patch and retry.
    #[error("revision conflict: current_seq={}", .0.current_seq)]
    Conflict(ConflictProblemDto),
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("decode error in {context}: {source}")]
    Decode {
        context: &'static str,
        source: serde_json::Error,
    },
}

/// Result of a confirmed Trash purge request.
#[derive(Debug, Clone)]
pub enum PurgeTrashResult {
    /// The purge and any required cleanup are complete.
    Complete,
    /// Database deletion committed, while cleanup is still pending or retryable.
    Pending(PurgeStatusDtoResponse),
}

/// A pending request built through one of the verb helpers.
///
/// Delegates the builder methods the client actually uses (`header`, `json`,
/// `body`) and defers the actual send to [`AtlasClient::send_with_retry`], so
/// every request goes through the 429-retry path without changing call sites.
#[must_use = "a Req does nothing until `.send().await` is awaited"]
struct Req<'a> {
    client: &'a AtlasClient,
    builder: reqwest::RequestBuilder,
}

impl Req<'_> {
    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.builder = self.builder.header(name, value.into());
        self
    }

    fn json<T: serde::Serialize + ?Sized>(mut self, json: &T) -> Self {
        self.builder = self.builder.json(json);
        self
    }

    fn body(mut self, body: impl Into<reqwest::Body>) -> Self {
        self.builder = self.builder.body(body);
        self
    }

    /// Overrides the client-level total timeout for this single request.
    fn timeout(mut self, timeout: Duration) -> Self {
        self.builder = self.builder.timeout(timeout);
        self
    }

    async fn send(self) -> Result<reqwest::Response, ClientError> {
        self.client.send_with_retry(self.builder).await
    }
}

/// Parses a `Retry-After` header value (delta-seconds) into a bounded wait.
///
/// The Atlas server emits an integer number of seconds. Anything missing or
/// unparseable falls back to one second. The result is clamped to
/// `[MIN_RETRY_WAIT, MAX_RETRY_WAIT]` so a hostile or misconfigured value cannot
/// make the client sleep indefinitely or busy-loop.
fn parse_retry_after(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(1);

    Duration::from_secs(secs).clamp(MIN_RETRY_WAIT, MAX_RETRY_WAIT)
}

/// The three REG-5 components whose routes this client addresses. The
/// variants' wire strings are the `/api/v2/<component>` mount segments
/// (`atlas_server::lib::app`); `atlas_server`'s route-contract test asserts
/// they are exactly the registry's API-declaring stable ids, so this enum
/// cannot drift from ownership even though this crate cannot see the
/// registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Platform,
    Custos,
    Acta,
}

impl Component {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Custos => "custos",
            Self::Acta => "acta",
        }
    }
}

// api-path-guard:off
const V2_PREFIX: &str = "/api/v2";
// api-path-guard:on

pub struct AtlasClient {
    base_url: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl AtlasClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        // Building with timeouts can only fail on TLS/resolver misconfiguration;
        // fall back to the default client rather than change the signature.
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            base_url: base_url.into(),
            http,
            token: None,
        }
    }

    /// Constructs a client that reuses an existing `reqwest::Client` connection pool.
    ///
    /// `reqwest::Client` is Arc-backed internally, so cloning it is cheap — this
    /// constructor takes ownership of the caller's clone and avoids spawning a new
    /// DNS resolver or TLS stack for each logical atlas_mcp session.
    pub fn with_shared_pool(
        pool: reqwest::Client,
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            http: pool,
            token: Some(token.into()),
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = Some(token.into());
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    /// Borrowing sub-client for every method mounted at `/api/v2/custos`
    /// (D1). Carries no state of its own, so authentication and CSRF
    /// configuration stay single-point on `self` (INV-SINGLE-AUTH-CONFIG).
    pub fn custos(&self) -> Custos<'_> {
        Custos(self)
    }

    /// Borrowing sub-client for every method mounted at `/api/v2/acta`
    /// (D1). Carries no state of its own, so authentication and CSRF
    /// configuration stay single-point on `self` (INV-SINGLE-AUTH-CONFIG).
    pub fn acta(&self) -> Acta<'_> {
        Acta(self)
    }

    /// Borrowing sub-client for every method mounted at `/api/v2/platform`
    /// (D1). Carries no state of its own, so authentication and CSRF
    /// configuration stay single-point on `self` (INV-SINGLE-AUTH-CONFIG).
    pub fn platform(&self) -> Platform<'_> {
        Platform(self)
    }

    /// Resolves `component`'s `/api/v2/<component>` mount plus `relative`
    /// into an absolute URL. The only place a component namespace is
    /// assembled into a concrete URL inside this crate.
    fn mounted(&self, component: Component, relative: &str) -> String {
        format!(
            "{}{V2_PREFIX}/{}{relative}",
            self.base_url,
            component.as_str()
        )
    }

    fn get(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.get(self.mounted(component, relative)))
    }

    fn post(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.post(self.mounted(component, relative)))
    }

    fn patch(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.patch(self.mounted(component, relative)))
    }

    fn put(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.put(self.mounted(component, relative)))
    }

    fn delete(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.delete(self.mounted(component, relative)))
    }

    /// The one composition-root route this client calls outside every
    /// component nest (`router_audit::ROOT_LEVEL_PATHS`): `GET /health`.
    /// Kept as a distinct method rather than a `Component` argument because a
    /// root-level path has no owning namespace on the wire.
    fn root_get(&self, path: &str) -> Req<'_> {
        self.request(self.http.get(format!("{}{}", self.base_url, path)))
    }

    /// Wraps a raw `RequestBuilder`, applying bearer auth, into a retry-aware `Req`.
    fn request(&self, mut builder: reqwest::RequestBuilder) -> Req<'_> {
        if let Some(token) = &self.token {
            builder = builder.bearer_auth(token);
        }
        Req {
            client: self,
            builder,
        }
    }

    /// Sends a request, transparently retrying on HTTP 429.
    ///
    /// On a `429 Too Many Requests` the server's per-principal rate limit was hit;
    /// the response carries a `Retry-After` interval. Bulk callers (the CLI and
    /// MCP server) would otherwise fail on the first rejection, so the client
    /// waits for that interval and retries up to `MAX_RATE_LIMIT_RETRIES` times.
    ///
    /// Requests whose body cannot be cloned (streaming bodies) are sent once with
    /// no retry, since replaying them is not possible.
    async fn send_with_retry(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let mut attempt: u32 = 0;

        loop {
            let attempt_builder = match request.try_clone() {
                Some(clone) => clone,
                None => return Ok(request.send().await?),
            };

            let response = attempt_builder.send().await?;

            if response.status().as_u16() == 429 && attempt < MAX_RATE_LIMIT_RETRIES {
                let wait = parse_retry_after(
                    response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok()),
                );
                attempt += 1;
                tokio::time::sleep(wait).await;
                continue;
            }

            return Ok(response);
        }
    }

    async fn decode_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        context: &'static str,
    ) -> Result<T, ClientError> {
        if !response.status().is_success() {
            let problem: ProblemDetails = response
                .json()
                .await
                .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
            return Err(ClientError::Api(problem));
        }

        let body = response.bytes().await?;
        serde_json::from_slice(&body).map_err(|source| ClientError::Decode { context, source })
    }

    /// `GET /health`
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let response = self.root_get("/health").send().await?;
        self.decode_response(response, "health").await
    }

    /// `POST /api/v2/custos/auth/login`
    ///
    /// On success, stores the returned session token in `self.token`.
    pub async fn login(&mut self, body: LoginRequest) -> Result<LoginResponse, ClientError> {
        let response = self
            .post(Component::Custos, "/auth/login")
            .json(&body)
            .send()
            .await?;
        let login: LoginResponse = self.decode_response(response, "login").await?;
        self.token = Some(login.token.clone());
        Ok(login)
    }
}

/// Percent-encodes characters that are not safe in a query-string value.
fn hex_nibble(n: u8) -> char {
    char::from_digit(n as u32, 16)
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('0')
}

fn encode_query_value(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf);
                bytes
                    .bytes()
                    .flat_map(|b| vec!['%', hex_nibble(b >> 4), hex_nibble(b & 0x0f)])
                    .collect::<Vec<_>>()
            }
        })
        .collect()
}

fn build_audit_path(
    base: &str,
    actor: Option<&str>,
    action: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(a) = actor {
        params.push(format!("actor={a}"));
    }
    if let Some(a) = action {
        params.push(format!("action={}", encode_query_value(a)));
    }
    if let Some(f) = from {
        params.push(format!("from={}", encode_query_value(f)));
    }
    if let Some(t) = to {
        params.push(format!("to={}", encode_query_value(t)));
    }
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

fn build_paginated_path(base: &str, cursor: Option<&str>, limit: Option<u32>) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_api::dtos::{
        boards_tasks::{CreateReferenceRequest, CreateTaskRequest},
        documents::{
            DocumentContentRangeQuery, DocumentContentSearchRequest, DocumentMoveBatchRequest,
            DocumentMoveBatchResultDto,
        },
        lifecycle::{PurgeStatusDto, TrashKindDto},
    };

    #[test]
    fn construction_stores_base_url() {
        let client = AtlasClient::new("http://localhost:8080");
        assert_eq!(client.base_url(), "http://localhost:8080");
    }

    #[test]
    fn component_as_str_matches_the_registrys_stable_ids() {
        assert_eq!(Component::Platform.as_str(), "platform");
        assert_eq!(Component::Custos.as_str(), "custos");
        assert_eq!(Component::Acta.as_str(), "acta");
    }

    #[test]
    fn mounted_joins_base_url_v2_prefix_component_and_relative_path() {
        let client = AtlasClient::new("http://localhost:8080");
        assert_eq!(
            client.mounted(Component::Acta, "/workspaces/{ws}/tasks"),
            "http://localhost:8080/api/v2/acta/workspaces/{ws}/tasks"
        );
        assert_eq!(
            client.mounted(Component::Custos, "/users"),
            "http://localhost:8080/api/v2/custos/users"
        );
        assert_eq!(
            client.mounted(Component::Platform, "/meta"),
            "http://localhost:8080/api/v2/platform/meta"
        );
    }

    #[tokio::test]
    async fn verb_methods_resolve_addresses_through_the_component_seam() {
        let (base_url, requests) = serve_once_observing("200 OK", "{}");
        let client = AtlasClient::new(base_url);
        let _ = client.get(Component::Acta, "/x").send().await;
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with("GET /api/v2/acta/x "),
            "expected V2-mounted request, got `{raw}`"
        );
    }

    #[tokio::test]
    async fn root_get_addresses_the_root_level_mount_with_no_component_or_api_prefix() {
        let (base_url, requests) = serve_once_observing("200 OK", "{}");
        let client = AtlasClient::new(base_url);
        let _ = client.root_get("/health").send().await;
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with("GET /health "),
            "expected an unprefixed root-level request, got `{raw}`"
        );
    }

    /// D2.6 — one runtime proof of the seam: five methods (one per verb),
    /// proving the seam mechanism rather than enumerating the client's
    /// surface (the source-walking contract test in `atlas_server` owns
    /// that).
    #[tokio::test]
    async fn every_verb_resolves_its_predicted_v2_mounted_url() {
        for (verb, expected_line) in [
            ("get", "GET /api/v2/acta/x "),
            ("post", "POST /api/v2/custos/y "),
            ("patch", "PATCH /api/v2/platform/z "),
            ("put", "PUT /api/v2/acta/w "),
            ("delete", "DELETE /api/v2/custos/q "),
        ] {
            let (base_url, requests) = serve_once_observing("200 OK", "{}");
            let client = AtlasClient::new(base_url);
            let _ = match verb {
                "get" => client.get(Component::Acta, "/x").send().await,
                "post" => client.post(Component::Custos, "/y").send().await,
                "patch" => client.patch(Component::Platform, "/z").send().await,
                "put" => client.put(Component::Acta, "/w").send().await,
                "delete" => client.delete(Component::Custos, "/q").send().await,
                _ => unreachable!(),
            };
            let raw = requests.recv().expect("mock server received request");
            assert!(
                raw.starts_with(expected_line),
                "verb `{verb}`: expected `{expected_line}`, got `{raw}`"
            );
        }
    }

    #[test]
    fn api_error_display_keeps_status_and_title_prefix() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422);
        let message = ClientError::Api(problem).to_string();
        assert_eq!(message, "API error 422: Invalid input");
    }

    #[test]
    fn api_error_display_includes_detail_and_hint_when_present() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422)
            .with_detail("title must not be empty")
            .with_hint("provide a non-empty title");
        let message = ClientError::Api(problem).to_string();
        assert_eq!(
            message,
            "API error 422: Invalid input — title must not be empty \
             (hint: provide a non-empty title)"
        );
    }

    #[test]
    fn api_error_display_includes_detail_without_hint() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422)
            .with_detail("title must not be empty");
        let message = ClientError::Api(problem).to_string();
        assert_eq!(
            message,
            "API error 422: Invalid input — title must not be empty"
        );
    }

    #[test]
    fn with_token_stores_token() {
        let client = AtlasClient::new("http://localhost:8080").with_token("test-token");
        assert!(client.token.is_some());
    }

    #[test]
    fn token_accessor_returns_none_initially() {
        let client = AtlasClient::new("http://localhost:8080");
        assert!(client.token().is_none());
    }

    #[test]
    fn encode_query_value_encodes_spaces() {
        let encoded = encode_query_value("hello world");
        assert!(encoded.contains("%20") || !encoded.contains(' '));
    }

    #[test]
    fn encode_query_value_preserves_alphanumeric() {
        let encoded = encode_query_value("abc123");
        assert_eq!(encoded, "abc123");
    }

    #[test]
    fn parse_retry_after_reads_delta_seconds() {
        assert_eq!(parse_retry_after(Some("5")), Duration::from_secs(5));
        assert_eq!(parse_retry_after(Some("  3 ")), Duration::from_secs(3));
    }

    #[test]
    fn parse_retry_after_defaults_to_one_second_when_absent_or_invalid() {
        assert_eq!(parse_retry_after(None), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("soon")), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("")), Duration::from_secs(1));
    }

    #[test]
    fn parse_retry_after_clamps_to_bounds() {
        assert_eq!(parse_retry_after(Some("0")), MIN_RETRY_WAIT);
        assert_eq!(parse_retry_after(Some("9999")), MAX_RETRY_WAIT);
    }

    fn serve_once(status: &'static str, body: &'static str) -> String {
        serve_once_observing(status, body).0
    }

    fn serve_once_observing(
        status: &'static str,
        body: impl Into<String> + Send + 'static,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let body = body.into();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let length = stream.read(&mut request).unwrap();
            let request = request.get(..length).unwrap_or_default();
            let _ = request_tx.send(String::from_utf8_lossy(request).into_owned());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}"), request_rx)
    }

    async fn assert_comment_attachment_request<T>(
        requests: std::sync::mpsc::Receiver<String>,
        expected_prefix: &str,
        request: impl std::future::Future<Output = Result<T, ClientError>>,
    ) -> T {
        let result = request.await.expect("client lifecycle request succeeds");
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with(expected_prefix),
            "expected `{expected_prefix}`, received `{raw}`"
        );
        result
    }

    async fn assert_comment_attachment_request_with_headers<T>(
        requests: std::sync::mpsc::Receiver<String>,
        expected_prefix: &str,
        expected_headers: &[&str],
        request: impl std::future::Future<Output = Result<T, ClientError>>,
    ) -> T {
        let result = request.await.expect("client lifecycle request succeeds");
        let raw = requests.recv().expect("mock server received request");

        assert!(
            raw.starts_with(expected_prefix),
            "expected `{expected_prefix}`, received `{raw}`"
        );

        for expected_header in expected_headers {
            assert!(
                raw.contains(expected_header),
                "expected request to contain `{expected_header}`, received `{raw}`"
            );
        }

        result
    }

    #[tokio::test]
    async fn server_meta_preserves_components_and_decode_context() {
        let absent_components = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"components":[]}"#,
        ));
        assert!(
            absent_components
                .platform()
                .server_meta()
                .await
                .unwrap()
                .components
                .is_empty()
        );

        let with_components = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"components":[{"stable_id":"platform","kind":"platform-service","contract_version":1}]}"#,
        ));
        let meta = with_components.platform().server_meta().await.unwrap();
        assert_eq!(meta.components.len(), 1);
        let component = meta.components.first().expect("one component");
        assert_eq!(component.stable_id, "platform");
        assert_eq!(component.kind, "platform-service");
        assert_eq!(component.contract_version, 1);

        let malformed = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"components":"not-an-array"}"#,
        ));
        assert!(matches!(
            malformed.platform().server_meta().await,
            Err(ClientError::Decode {
                context: "server_meta",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn server_meta_preserves_api_and_transport_errors() {
        let api = AtlasClient::new(serve_once(
            "503 Service Unavailable",
            r#"{"type":"urn:atlas:error","title":"Unavailable","status":503}"#,
        ));
        assert!(matches!(
            api.platform().server_meta().await,
            Err(ClientError::Api(_))
        ));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let transport = AtlasClient::new(format!("http://{address}"));
        assert!(matches!(
            transport.platform().server_meta().await,
            Err(ClientError::Transport(_))
        ));
    }

    #[tokio::test]
    async fn task_creation_methods_decode_reference_envelopes_and_preserve_task_compatibility() {
        const BOARD_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000010");
        const DOCUMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000020");
        const RESPONSE: &str = r#"{"task":{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","board_id":"00000000-0000-0000-0000-000000000010","column_id":"00000000-0000-0000-0000-000000000004","readable_id":"ATL-1","title":"Source","description":"","labels":[],"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","board_name":"Board","column_name":"Todo"},"references":[{"id":"00000000-0000-0000-0000-000000000006","kind":"relates","target_readable_id":"ATL-2","target_title":"Current target task","target_resolved":true,"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z"},{"id":"00000000-0000-0000-0000-000000000007","kind":"spec","target_document_id":"00000000-0000-0000-0000-000000000020","target_title":"Current target document","target_resolved":true,"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z"}]}"#;

        let request = CreateTaskRequest {
            column_id: uuid::uuid!("00000000-0000-0000-0000-000000000004"),
            title: "Source".into(),
            description: None,
            properties: None,
            before: None,
            after: None,
            references: vec![
                CreateReferenceRequest {
                    kind: "relates".into(),
                    target_task_readable_id: Some("ATL-2".into()),
                    target_document_id: None,
                },
                CreateReferenceRequest {
                    kind: "spec".into(),
                    target_task_readable_id: None,
                    target_document_id: Some(DOCUMENT_ID),
                },
            ],
        };

        let (base_url, requests) = serve_once_observing("201 Created", RESPONSE);
        let created = AtlasClient::new(base_url)
            .acta()
            .create_task_with_references("ws", BOARD_ID, request.clone())
            .await
            .expect("reference-aware task creation succeeds");

        assert_eq!(created.task.readable_id, "ATL-1");
        assert_eq!(created.references.len(), 2);
        assert_eq!(
            created
                .references
                .first()
                .and_then(|reference| reference.target_title.as_deref()),
            Some("Current target task")
        );
        assert_eq!(
            created
                .references
                .last()
                .and_then(|reference| reference.target_title.as_deref()),
            Some("Current target document")
        );

        let raw = requests.recv().expect("mock server received request");
        assert!(raw.starts_with(
            "POST /api/v2/acta/workspaces/ws/boards/00000000-0000-0000-0000-000000000010/tasks "
        ));
        assert!(raw.contains("\"target_task_readable_id\":\"ATL-2\""));
        assert!(raw.contains("\"target_document_id\":\"00000000-0000-0000-0000-000000000020\""));

        let task = AtlasClient::new(serve_once("201 Created", RESPONSE))
            .acta()
            .create_task("ws", BOARD_ID, request)
            .await
            .expect("task-only compatibility method succeeds");

        assert_eq!(task.readable_id, "ATL-1");
        assert_eq!(task.title, "Source");
    }

    #[tokio::test]
    async fn comment_attachment_lifecycle_methods_use_canonical_task_and_document_routes() {
        const COMMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const ATTACHMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const ATTACHMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000002","comment_id":"00000000-0000-0000-0000-000000000001","file_name":"note.txt","content_type":"text/plain","size_bytes":2,"sha256":"digest","actor":null,"created_at":"2026-01-01T00:00:00Z"}"#;

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        let attachment = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.acta().upload_task_comment_attachment("ws", "ATL-1", COMMENT_ID, "note.txt", "text/plain", b"ok".to_vec()),
        )
        .await;
        assert_eq!(attachment.id, ATTACHMENT_ID);

        let (base_url, requests) = serve_once_observing("200 OK", format!("[{ATTACHMENT}]"));
        let client = AtlasClient::new(base_url);
        let attachments = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.acta().list_task_comment_attachments("ws", "ATL-1", COMMENT_ID),
        )
        .await;
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments.first().map(|attachment| attachment.id),
            Some(ATTACHMENT_ID)
        );

        let (base_url, requests) = serve_once_observing("200 OK", "ok");
        let client = AtlasClient::new(base_url);
        let (data, _) = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002/content ",
            client.acta().download_task_comment_attachment("ws", "ATL-1", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;
        assert_eq!(data, b"ok");

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.acta().delete_task_comment_attachment("ws", "ATL-1", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.acta().upload_document_comment_attachment("ws", "note", COMMENT_ID, "note.txt", "text/plain", b"ok".to_vec()),
        )
        .await;

        let (base_url, requests) = serve_once_observing("200 OK", format!("[{ATTACHMENT}]"));
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.acta().list_document_comment_attachments("ws", "note", COMMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("200 OK", "ok");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.acta().download_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.acta().delete_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;
    }

    #[tokio::test]
    async fn comment_draft_methods_use_frozen_routes_tokens_and_transports() {
        const DRAFT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const CREATE_TOKEN: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const UPLOAD_TOKEN: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
        const DRAFT: &str =
            r#"{"id":"00000000-0000-0000-0000-000000000001","expires_at":"2026-01-02T00:00:00Z"}"#;
        const ATTACHMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000004","comment_id":"00000000-0000-0000-0000-000000000001","file_name":"note.txt","content_type":"text/plain","size_bytes":2,"sha256":"digest","actor":null,"created_at":"2026-01-01T00:00:00Z","url":"/attachment","markdown":"[note.txt](/attachment)"}"#;

        let (base_url, requests) = serve_once_observing("201 Created", DRAFT);
        let client = AtlasClient::new(base_url);
        let task_draft = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts ",
            &["x-create-token: 00000000-0000-0000-0000-000000000002"],
            client
                .acta()
                .create_task_comment_draft("ws", "ATL-1", CREATE_TOKEN),
        )
        .await;
        assert_eq!(task_draft.id, DRAFT_ID);

        let (base_url, requests) = serve_once_observing("200 OK", DRAFT);
        let client = AtlasClient::new(base_url);
        let document_draft = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comment-drafts ",
            &["x-create-token: 00000000-0000-0000-0000-000000000002"],
            client
                .acta()
                .create_document_comment_draft("ws", "note", CREATE_TOKEN),
        )
        .await;
        assert_eq!(
            document_draft.expires_at.to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        let attachment = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts/00000000-0000-0000-0000-000000000001/attachments ",
            &[
                "x-upload-token: 00000000-0000-0000-0000-000000000003",
                "multipart/form-data; boundary=atlasboundary",
            ],
            client.acta().upload_task_draft_attachment(
                "ws", "ATL-1", DRAFT_ID, UPLOAD_TOKEN, "note.txt", "text/plain", b"ok".to_vec(),
            ),
        )
        .await;
        assert_eq!(
            attachment.markdown.as_deref(),
            Some("[note.txt](/attachment)")
        );

        let (base_url, requests) = serve_once_observing("200 OK", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comment-drafts/00000000-0000-0000-0000-000000000001/attachments ",
            &[
                "x-upload-token: 00000000-0000-0000-0000-000000000003",
                "x-file-name: note.txt",
                "content-type: text/plain",
            ],
            client.acta().upload_document_draft_attachment(
                "ws", "note", DRAFT_ID, UPLOAD_TOKEN, "note.txt", "text/plain", b"ok".to_vec(),
            ),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts/00000000-0000-0000-0000-000000000001 ",
            client.acta().cancel_task_comment_draft("ws", "ATL-1", DRAFT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/documents/note/comment-drafts/00000000-0000-0000-0000-000000000001 ",
            client.acta().cancel_document_comment_draft("ws", "note", DRAFT_ID),
        )
        .await;
    }

    #[tokio::test]
    async fn comment_attachment_lifecycle_methods_preserve_api_decode_and_transport_errors() {
        const COMMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const ATTACHMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");

        let api = AtlasClient::new(serve_once(
            "403 Forbidden",
            r#"{"type":"urn:atlas:error:forbidden","title":"Forbidden","status":403}"#,
        ));
        assert!(matches!(
            api.acta()
                .upload_task_comment_attachment(
                    "ws",
                    "ATL-1",
                    COMMENT_ID,
                    "note.txt",
                    "text/plain",
                    b"ok".to_vec(),
                )
                .await,
            Err(ClientError::Api(_))
        ));

        let decode = AtlasClient::new(serve_once("200 OK", "not attachment metadata"));
        assert!(matches!(
            decode
                .acta()
                .list_document_comment_attachments("ws", "note", COMMENT_ID)
                .await,
            Err(ClientError::Decode {
                context: "list_document_comment_attachments",
                ..
            })
        ));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let transport = AtlasClient::new(format!("http://{address}"));
        assert!(matches!(
            transport
                .acta()
                .delete_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID)
                .await,
            Err(ClientError::Transport(_))
        ));
    }

    #[tokio::test]
    async fn trash_lifecycle_methods_use_typed_admin_routes_and_status_mapping() {
        const TARGET_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const OPERATION_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const PAGE: &str = r#"{"items":[{"workspace_id":"00000000-0000-0000-0000-000000000003","kind":"document","target_id":"00000000-0000-0000-0000-000000000001","deleted_at":"2026-07-22T00:00:00Z"}],"next_cursor":"next","has_more":true}"#;
        const STATUS: &str = r#"{"operation_id":"00000000-0000-0000-0000-000000000002","kind":"document","target_id":"00000000-0000-0000-0000-000000000001","status":"cleanup_pending","attempts":2}"#;

        let (base_url, requests) = serve_once_observing("200 OK", PAGE);
        let client = AtlasClient::new(base_url);
        let page = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/admin/trash?workspace_id=00000000-0000-0000-0000-000000000003&kind=document&cursor=next&limit=20 ",
            client.acta().list_trash(
                Some(uuid::uuid!("00000000-0000-0000-0000-000000000003")),
                Some(TrashKindDto::Document),
                Some("next"),
                Some(20),
            ),
        )
        .await;
        assert_eq!(
            page.items.first().map(|item| item.target_id),
            Some(TARGET_ID)
        );

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/restore ",
            client
                .acta()
                .restore_trash(TrashKindDto::Document, TARGET_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("202 Accepted", STATUS);
        let client = AtlasClient::new(base_url);
        let response = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/purge ",
            client
                .acta()
                .purge_trash(TrashKindDto::Document, TARGET_ID, true),
        )
        .await;
        assert!(matches!(
            response,
            PurgeTrashResult::Pending(PurgeStatusDtoResponse {
                status: PurgeStatusDto::CleanupPending,
                ..
            })
        ));

        let (base_url, requests) = serve_once_observing("200 OK", STATUS);
        let client = AtlasClient::new(base_url);
        let status = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/admin/trash/purges/00000000-0000-0000-0000-000000000002 ",
            client.acta().get_purge_status(OPERATION_ID),
        )
        .await;
        assert_eq!(status.operation_id, OPERATION_ID);

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        let complete = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/purge ",
            client
                .acta()
                .purge_trash(TrashKindDto::Document, TARGET_ID, true),
        )
        .await;
        assert!(matches!(complete, PurgeTrashResult::Complete));

        let denied = AtlasClient::new(serve_once(
            "403 Forbidden",
            r#"{"type":"urn:atlas:error:forbidden","title":"Forbidden","status":403}"#,
        ));
        assert!(matches!(
            denied.acta().restore_trash(TrashKindDto::Document, TARGET_ID).await,
            Err(ClientError::Api(problem)) if problem.status == 403
        ));
    }

    #[tokio::test]
    async fn bounded_document_methods_serialize_requests_and_decode_continuations() {
        const COMPACT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        const RANGE: &str = r#"{"head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"lines":[{"line_number":3,"text":"third"}],"byte_count":5,"has_more":true,"continuation":"range-next"}"#;
        const SEARCH: &str = r#"{"head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"matches":[{"line_number":5,"preview":"needle"}],"byte_count":6,"has_more":true,"continuation":"search-next"}"#;

        let (base_url, requests) = serve_once_observing("200 OK", COMPACT);
        let compact = AtlasClient::new(base_url)
            .acta()
            .get_document_compact("ws", "note")
            .await
            .expect("compact document request succeeds");
        assert_eq!(compact.title, "Note");
        assert_eq!(compact.head_seq, 4);
        assert!(
            requests
                .recv()
                .expect("mock server received compact request")
                .starts_with("GET /api/v2/acta/workspaces/ws/documents/note/compact ")
        );

        let (base_url, requests) = serve_once_observing("200 OK", RANGE);
        let range = AtlasClient::new(base_url)
            .acta()
            .get_document_content_range(
                "ws",
                "note",
                DocumentContentRangeQuery {
                    start_line: Some(3),
                    end_line: Some(8),
                    line_limit: Some(2),
                    byte_limit: Some(64),
                    continuation: Some("range token".into()),
                },
            )
            .await
            .expect("bounded range request succeeds");
        assert_eq!(range.lines.first().map(|line| line.line_number), Some(3));
        assert_eq!(range.continuation.as_deref(), Some("range-next"));
        let range_request = requests.recv().expect("mock server received range request");
        assert!(range_request.starts_with(
            "GET /api/v2/acta/workspaces/ws/documents/note/content/range?start_line=3&end_line=8&line_limit=2&byte_limit=64&continuation=range%20token "
        ));

        let (base_url, requests) = serve_once_observing("200 OK", SEARCH);
        let search = AtlasClient::new(base_url)
            .acta()
            .search_document_content(
                "ws",
                "note",
                DocumentContentSearchRequest {
                    start_line: Some(2),
                    end_line: Some(9),
                    query: "a+b".into(),
                    mode: Some(atlas_api::dtos::documents::DocumentSearchMode::Pattern),
                    match_limit: Some(3),
                    byte_limit: Some(128),
                    continuation: Some("search-next".into()),
                },
            )
            .await
            .expect("bounded search request succeeds");
        assert_eq!(
            search
                .matches
                .first()
                .map(|matched| matched.preview.as_str()),
            Some("needle")
        );
        assert_eq!(search.continuation.as_deref(), Some("search-next"));
        let search_request = requests
            .recv()
            .expect("mock server received search request");
        assert!(
            search_request
                .starts_with("POST /api/v2/acta/workspaces/ws/documents/note/content/search ")
        );
        assert!(search_request.contains("\"query\":\"a+b\""));
        assert!(search_request.contains("\"mode\":\"pattern\""));
        assert!(search_request.contains("\"continuation\":\"search-next\""));
    }

    #[tokio::test]
    async fn bounded_document_range_omits_absent_query_fields_and_preserves_api_errors() {
        let (base_url, requests) = serve_once_observing(
            "409 Conflict",
            r#"{"type":"urn:atlas:error:conflict","title":"Conflict","status":409,"hint":"restart the range read"}"#,
        );
        let error = AtlasClient::new(base_url)
            .acta()
            .get_document_content_range("ws", "note", DocumentContentRangeQuery::default())
            .await
            .expect_err("stale continuation response is an API error");
        assert!(matches!(error, ClientError::Api(problem) if problem.status == 409));
        assert!(
            requests
                .recv()
                .expect("mock server received range request")
                .starts_with("GET /api/v2/acta/workspaces/ws/documents/note/content/range ")
        );
    }

    #[tokio::test]
    async fn document_line_edit_serializes_tagged_request_and_preserves_conflicts() {
        const DOCUMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let request = atlas_api::dtos::documents::DocumentContentEditRequest {
            base_revision_id: "00000000-0000-0000-0000-000000000003"
                .parse()
                .expect("test revision UUID is valid"),
            edit: atlas_api::dtos::documents::DocumentLineEditRequest::Replace {
                start: 2,
                end: 2,
                content: "updated".into(),
            },
        };

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        let document = AtlasClient::new(base_url)
            .acta()
            .edit_document_content_range("ws", "note", request.clone())
            .await
            .expect("line edit succeeds");
        assert_eq!(document.head_seq, 5);
        assert_eq!(
            document.head_revision_id.to_string(),
            "00000000-0000-0000-0000-000000000003"
        );

        let sent = requests.recv().expect("mock server received edit request");
        assert!(sent.starts_with("PATCH /api/v2/acta/workspaces/ws/documents/note/content/range "));
        assert!(sent.contains("\"base_revision_id\":\"00000000-0000-0000-0000-000000000003\""));
        assert!(sent.contains("\"operation\":\"replace\""));
        assert!(sent.contains("\"start\":2"));
        assert!(sent.contains("\"end\":2"));
        assert!(sent.contains("\"content\":\"updated\""));

        let conflict = AtlasClient::new(serve_once(
            "409 Conflict",
            r#"{"type":"urn:atlas:error:conflict","title":"Conflict","status":409,"current_revision_id":"00000000-0000-0000-0000-000000000004","current_seq":6,"base_to_current_patch":"@@ -1 +1 @@"}"#,
        ))
        .acta().edit_document_content_range("ws", "note", request)
        .await
        .expect_err("stale line edit returns a typed conflict");
        assert!(matches!(conflict, ClientError::Conflict(problem)
            if problem.current_seq == 6
                && problem.base_to_current_patch == "@@ -1 +1 @@"));
    }

    #[tokio::test]
    async fn document_line_edit_serializes_exact_insert_and_delete_request_bodies() {
        const DOCUMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let base_revision_id = "00000000-0000-0000-0000-000000000003"
            .parse()
            .expect("test revision UUID is valid");

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        AtlasClient::new(base_url)
            .acta()
            .edit_document_content_range(
                "ws",
                "note",
                atlas_api::dtos::documents::DocumentContentEditRequest {
                    base_revision_id,
                    edit: atlas_api::dtos::documents::DocumentLineEditRequest::Insert {
                        position: 3,
                        content: "new line".into(),
                    },
                },
            )
            .await
            .expect("insert succeeds");
        let insert: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received insert request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("insert body is JSON");
        assert_eq!(
            insert,
            serde_json::json!({
                "base_revision_id": "00000000-0000-0000-0000-000000000003",
                "operation": "insert",
                "position": 3,
                "content": "new line",
            })
        );

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        AtlasClient::new(base_url)
            .acta()
            .edit_document_content_range(
                "ws",
                "note",
                atlas_api::dtos::documents::DocumentContentEditRequest {
                    base_revision_id,
                    edit: atlas_api::dtos::documents::DocumentLineEditRequest::Delete {
                        start: 2,
                        end: 4,
                    },
                },
            )
            .await
            .expect("delete succeeds");
        let delete: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received delete request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("delete body is JSON");
        assert_eq!(
            delete,
            serde_json::json!({
                "base_revision_id": "00000000-0000-0000-0000-000000000003",
                "operation": "delete",
                "start": 2,
                "end": 4,
            })
        );
    }

    #[tokio::test]
    async fn document_line_edit_preserves_validation_hints() {
        let request = atlas_api::dtos::documents::DocumentContentEditRequest {
            base_revision_id: "00000000-0000-0000-0000-000000000003"
                .parse()
                .expect("test revision UUID is valid"),
            edit: atlas_api::dtos::documents::DocumentLineEditRequest::Insert {
                position: 1,
                content: "new first line".into(),
            },
        };
        let error = AtlasClient::new(serve_once(
            "422 Unprocessable Content",
            r#"{"type":"urn:atlas:error:validation","title":"Validation failed","status":422,"hint":"position must be within the document line range"}"#,
        ))
        .acta().edit_document_content_range("ws", "note", request)
        .await
        .expect_err("invalid line edit returns the server validation problem");

        assert!(matches!(
            error,
            ClientError::Api(problem)
                if problem.status == 422
                    && problem.hint.as_deref()
                        == Some("position must be within the document line range")
        ));
    }

    #[tokio::test]
    async fn document_move_batch_serializes_ordered_moves_and_decodes_mixed_outcomes() {
        const RESPONSE: &str = r#"[{"outcome":"success","index":0,"document":{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","folder_id":"00000000-0000-0000-0000-000000000004","slug":"first","title":"Current first title","head_revision_id":"00000000-0000-0000-0000-000000000005","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}},{"outcome":"problem","index":1,"status":404,"type":"urn:atlas:error:not-found","title":"Not found","hint":"document is unavailable"},{"outcome":"success","index":2,"document":{"id":"00000000-0000-0000-0000-000000000006","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","folder_id":null,"slug":"third","title":"Current third title","head_revision_id":"00000000-0000-0000-0000-000000000007","head_seq":7,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}}]"#;

        let destination_folder = uuid::uuid!("00000000-0000-0000-0000-000000000004");
        let request = DocumentMoveBatchRequest {
            moves: vec![
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "first".into(),
                    folder_id: Some(destination_folder),
                },
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "missing".into(),
                    folder_id: Some(destination_folder),
                },
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "third".into(),
                    folder_id: None,
                },
            ],
        };
        let (base_url, requests) = serve_once_observing("200 OK", RESPONSE);

        let outcomes = AtlasClient::new(base_url)
            .acta()
            .move_documents_batch("ws", request)
            .await
            .expect("batch move succeeds with per-item outcomes");

        assert!(matches!(
            outcomes.as_slice(),
            [
                DocumentMoveBatchResultDto::Success { index: 0, document },
                DocumentMoveBatchResultDto::Problem {
                    index: 1,
                    status: 404,
                    r#type,
                    title,
                    hint: Some(hint),
                },
                DocumentMoveBatchResultDto::Success { index: 2, .. },
            ] if document.title == "Current first title"
                && r#type == "urn:atlas:error:not-found"
                && title == "Not found"
                && hint == "document is unavailable"
        ));

        let sent: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received batch move request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("batch move body is JSON");
        assert_eq!(
            sent,
            serde_json::json!({
                "moves": [
                    {"source_document": "first", "folder_id": destination_folder},
                    {"source_document": "missing", "folder_id": destination_folder},
                    {"source_document": "third", "folder_id": null},
                ]
            })
        );
    }
}
