use reqwest::{
    Body, Method, Request, Url,
    header::{ACCEPT, AUTHORIZATION, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::{IpAddr, Ipv6Addr},
    path::Path,
    sync::{Arc, Mutex},
};
use thiserror::Error;

#[cfg(feature = "desktop-gate")]
pub mod gate;

const KEYRING_SERVICE: &str = "atlas-desktop";
const ACTIVE_IDENTITY_ACCOUNT_PREFIX: &str = "active-identity:";

pub trait TransportFactory {
    fn client(&self) -> Result<reqwest::Client, reqwest::Error>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReqwestTransportFactory;

impl ReqwestTransportFactory {
    pub fn system() -> Self {
        Self
    }
}

impl TransportFactory for ReqwestTransportFactory {
    fn client(&self) -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder().build()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionScope {
    origin: String,
    identity: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopConfiguration {
    origin: String,
}

impl DesktopConfiguration {
    pub fn from_selected_origin(origin: &str) -> Result<Self, DesktopError> {
        Ok(Self {
            origin: canonical_origin(origin)?,
        })
    }

    pub fn load(directory: &Path) -> Result<Self, DesktopError> {
        let configuration = fs::read_to_string(directory.join("desktop.json"))
            .map_err(|_| DesktopError::ConfigurationUnavailable)?;
        let configuration = serde_json::from_str::<Self>(&configuration)
            .map_err(|_| DesktopError::ConfigurationUnavailable)?;

        Self::from_selected_origin(&configuration.origin)
    }

    pub fn save(&self, directory: &Path) -> Result<(), DesktopError> {
        fs::create_dir_all(directory).map_err(|_| DesktopError::ConfigurationUnavailable)?;
        let configuration =
            serde_json::to_string(self).map_err(|_| DesktopError::ConfigurationUnavailable)?;

        fs::write(directory.join("desktop.json"), format!("{configuration}\n"))
            .map_err(|_| DesktopError::ConfigurationUnavailable)
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }
}

impl SessionScope {
    pub fn new(origin: &str, identity: &str) -> Result<Self, DesktopError> {
        let origin = canonical_origin(origin)?;

        if identity.is_empty() || identity.len() > 128 {
            return Err(DesktopError::InvalidIdentity);
        }

        Ok(Self {
            origin,
            identity: identity.to_owned(),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    fn key(&self) -> String {
        format!("{}:{}", self.origin, self.identity)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    Rest,
    Sse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopApiRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub fn build_authenticated_api_request(
    origin: &str,
    bearer: &str,
    request: DesktopApiRequest,
) -> Result<Request, DesktopError> {
    let mut authenticated = build_authenticated_request(
        origin,
        &request.method,
        &request.path,
        bearer,
        TransportKind::Rest,
    )?;

    for (name, value) in request.headers {
        let name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| DesktopError::InvalidHeader)?;
        if matches!(
            name.as_str(),
            "authorization"
                | "cookie"
                | "host"
                | "content-length"
                | "connection"
                | "transfer-encoding"
        ) {
            return Err(DesktopError::InvalidHeader);
        }
        let value = HeaderValue::from_str(&value).map_err(|_| DesktopError::InvalidHeader)?;
        authenticated.headers_mut().append(name, value);
    }

    if !request.body.is_empty() {
        *authenticated.body_mut() = Some(Body::from(request.body));
    }

    Ok(authenticated)
}

pub fn build_authenticated_request(
    origin: &str,
    method: &str,
    path: &str,
    bearer: &str,
    transport: TransportKind,
) -> Result<Request, DesktopError> {
    let origin = canonical_origin(origin)?;

    validate_api_path(path)?;

    if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD") {
        return Err(DesktopError::InvalidMethod);
    }

    if bearer.is_empty() || bearer.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(DesktopError::InvalidBearer);
    }

    if !path.starts_with("/api/") || path.starts_with("//") {
        return Err(DesktopError::InvalidApiPath);
    }

    let url = Url::parse(&format!("{origin}{path}")).map_err(|_| DesktopError::InvalidApiPath)?;
    if !url.path().starts_with("/api/") {
        return Err(DesktopError::InvalidApiPath);
    }
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| DesktopError::InvalidMethod)?;
    let mut request = Request::new(method, url);
    let mut authorization = HeaderValue::from_str(&format!("Bearer {bearer}"))
        .map_err(|_| DesktopError::InvalidBearer)?;
    authorization.set_sensitive(true);
    request.headers_mut().insert(AUTHORIZATION, authorization);

    if transport == TransportKind::Sse {
        request
            .headers_mut()
            .insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    }

    Ok(request)
}

pub async fn execute_protected_rest(origin: &str, bearer: &str) -> Result<(), DesktopError> {
    let request =
        build_authenticated_request(origin, "GET", "/api/auth/me", bearer, TransportKind::Rest)?;
    let response = reqwest::Client::new()
        .execute(request)
        .await
        .map_err(|_| DesktopError::TransportUnavailable)?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(DesktopError::SessionInvalid)
    }
}

/// Revokes a bearer session through Atlas's public logout endpoint.
pub async fn execute_bearer_logout(
    client: &reqwest::Client,
    origin: &str,
    bearer: &str,
) -> Result<(), DesktopError> {
    let request = build_authenticated_request(
        origin,
        "POST",
        "/api/auth/logout",
        bearer,
        TransportKind::Rest,
    )?;
    let response = client
        .execute(request)
        .await
        .map_err(|_| DesktopError::TransportUnavailable)?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(DesktopError::SessionInvalid)
    }
}

pub async fn execute_bearer_sse(
    origin: &str,
    workspace: &str,
    bearer: &str,
) -> Result<(), DesktopError> {
    validate_workspace(workspace)?;
    let request = build_authenticated_request(
        origin,
        "GET",
        &format!("/api/workspaces/{workspace}/events"),
        bearer,
        TransportKind::Sse,
    )?;
    let response = reqwest::Client::new()
        .execute(request)
        .await
        .map_err(|_| DesktopError::TransportUnavailable)?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(DesktopError::SessionInvalid)
    }
}

pub async fn execute_workspace_sse(
    origin: &str,
    workspace: &str,
    bearer: &str,
) -> Result<String, DesktopError> {
    validate_workspace(workspace)?;
    let request = build_authenticated_request(
        origin,
        "GET",
        &format!("/api/workspaces/{workspace}/events"),
        bearer,
        TransportKind::Sse,
    )?;
    let response = reqwest::Client::new()
        .execute(request)
        .await
        .map_err(|_| DesktopError::TransportUnavailable)?;

    if !response.status().is_success() {
        return Err(DesktopError::SessionInvalid);
    }

    response
        .text()
        .await
        .map_err(|_| DesktopError::TransportUnavailable)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Authenticated,
    Unauthenticated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleAction {
    PurgeScopedCache(SessionScope),
    CancelTransportAndPurgeScopedCache(SessionScope),
}

impl LifecycleAction {
    pub fn scope(&self) -> &SessionScope {
        match self {
            Self::PurgeScopedCache(scope) | Self::CancelTransportAndPurgeScopedCache(scope) => {
                scope
            }
        }
    }

    pub fn cancels_transport(&self) -> bool {
        matches!(self, Self::CancelTransportAndPurgeScopedCache(_))
    }
}

pub trait SecretStore {
    fn store(&mut self, scope: &SessionScope, bearer: &str) -> Result<(), SecretStoreError>;
    fn load(&self, scope: &SessionScope) -> Result<String, SecretStoreError>;
    fn remove(&mut self, scope: &SessionScope) -> Result<(), SecretStoreError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretStoreError {
    #[error("secret storage is unavailable")]
    Unavailable,
}

#[derive(Default)]
pub struct SecretServiceStore;

impl SecretStore for SecretServiceStore {
    fn store(&mut self, scope: &SessionScope, bearer: &str) -> Result<(), SecretStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &scope.key())
            .map_err(|_| SecretStoreError::Unavailable)?;
        entry
            .set_password(bearer)
            .map_err(|_| SecretStoreError::Unavailable)
    }

    fn load(&self, scope: &SessionScope) -> Result<String, SecretStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &scope.key())
            .map_err(|_| SecretStoreError::Unavailable)?;
        entry
            .get_password()
            .map_err(|_| SecretStoreError::Unavailable)
    }

    fn remove(&mut self, scope: &SessionScope) -> Result<(), SecretStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &scope.key())
            .map_err(|_| SecretStoreError::Unavailable)?;
        entry
            .delete_credential()
            .map_err(|_| SecretStoreError::Unavailable)
    }
}

pub fn store_active_identity(origin: &str, identity: &str) -> Result<(), SecretStoreError> {
    let entry = keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{ACTIVE_IDENTITY_ACCOUNT_PREFIX}{origin}"),
    )
    .map_err(|_| SecretStoreError::Unavailable)?;
    entry
        .set_password(identity)
        .map_err(|_| SecretStoreError::Unavailable)
}

pub fn load_active_identity(origin: &str) -> Result<String, SecretStoreError> {
    let entry = keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{ACTIVE_IDENTITY_ACCOUNT_PREFIX}{origin}"),
    )
    .map_err(|_| SecretStoreError::Unavailable)?;
    entry
        .get_password()
        .map_err(|_| SecretStoreError::Unavailable)
}

pub fn clear_active_identity(origin: &str) -> Result<(), SecretStoreError> {
    let entry = keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{ACTIVE_IDENTITY_ACCOUNT_PREFIX}{origin}"),
    )
    .map_err(|_| SecretStoreError::Unavailable)?;
    entry
        .delete_credential()
        .map_err(|_| SecretStoreError::Unavailable)
}

#[derive(Clone, Default)]
pub struct InMemorySecretStore {
    entries: Arc<Mutex<HashMap<String, String>>>,
    locked: bool,
}

impl InMemorySecretStore {
    pub fn missing() -> Self {
        Self::default()
    }

    pub fn locked() -> Self {
        Self {
            entries: Arc::default(),
            locked: true,
        }
    }

    pub fn with_session(scope: SessionScope, bearer: &str) -> Self {
        let mut entries = HashMap::new();
        entries.insert(scope.key(), bearer.to_owned());
        Self {
            entries: Arc::new(Mutex::new(entries)),
            locked: false,
        }
    }

    pub fn remove(&mut self, scope: &SessionScope) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&scope.key());
        }
    }
}

impl SecretStore for InMemorySecretStore {
    fn store(&mut self, scope: &SessionScope, bearer: &str) -> Result<(), SecretStoreError> {
        if self.locked {
            return Err(SecretStoreError::Unavailable);
        }

        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .insert(scope.key(), bearer.to_owned());
        Ok(())
    }

    fn load(&self, scope: &SessionScope) -> Result<String, SecretStoreError> {
        if self.locked {
            return Err(SecretStoreError::Unavailable);
        }

        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .get(&scope.key())
            .cloned()
            .ok_or(SecretStoreError::Unavailable)
    }

    fn remove(&mut self, scope: &SessionScope) -> Result<(), SecretStoreError> {
        if self.locked {
            return Err(SecretStoreError::Unavailable);
        }

        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .remove(&scope.key())
            .map(|_| ())
            .ok_or(SecretStoreError::Unavailable)
    }
}

pub struct Lifecycle<S> {
    store: S,
    transport_active: bool,
    pending_action: Option<LifecycleAction>,
}

impl<S: SecretStore> Lifecycle<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            transport_active: false,
            pending_action: None,
        }
    }

    pub fn resume(&mut self, scope: &SessionScope) -> SessionState {
        if self.store.load(scope).is_ok() {
            self.transport_active = true;
            return SessionState::Authenticated;
        }

        self.transport_active = false;
        self.pending_action = Some(LifecycleAction::PurgeScopedCache(scope.clone()));
        SessionState::Unauthenticated
    }

    pub fn store_session(&mut self, scope: &SessionScope, bearer: &str) -> SessionState {
        if self.store.store(scope, bearer).is_ok() {
            self.transport_active = true;
            return SessionState::Authenticated;
        }

        self.transport_active = false;
        self.pending_action = Some(LifecycleAction::PurgeScopedCache(scope.clone()));
        SessionState::Unauthenticated
    }

    pub fn expire_or_revoke(&mut self, scope: &SessionScope) {
        self.transport_active = false;

        match self.store.remove(scope) {
            Ok(()) | Err(SecretStoreError::Unavailable) => {
                self.pending_action = Some(LifecycleAction::CancelTransportAndPurgeScopedCache(
                    scope.clone(),
                ));
            }
        }
    }

    pub fn transport_active(&self) -> bool {
        self.transport_active
    }

    pub fn take_action(&mut self) -> Option<LifecycleAction> {
        self.pending_action.take()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamFrame {
    LiveEnvelope(serde_json::Value),
    Resync,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamTermination {
    Reconnect,
    AuthLoss,
}

pub fn classify_workspace_stream_terminal(status: Option<u16>) -> StreamTermination {
    if status == Some(401) {
        StreamTermination::AuthLoss
    } else {
        StreamTermination::Reconnect
    }
}

/// Parses complete SSE frames and forwards each Atlas envelope without altering its shape.
pub fn process_workspace_sse_chunk<F>(
    pending: &mut String,
    chunk: &[u8],
    mut emit: F,
) -> Result<(), DesktopError>
where
    F: FnMut(StreamFrame) -> Result<(), DesktopError>,
{
    let chunk = std::str::from_utf8(chunk).map_err(|_| DesktopError::InvalidSseEvent)?;
    pending.push_str(chunk);

    while let Some(end) = pending.find("\n\n") {
        let frame = pending[..end].to_owned();
        pending.drain(..end + 2);

        let event_type = frame.lines().find_map(|line| line.strip_prefix("event: "));
        let data = frame.lines().find_map(|line| line.strip_prefix("data: "));

        if event_type == Some("resync") && data.is_none() {
            emit(StreamFrame::Resync)?;
            continue;
        }

        let data = data.ok_or(DesktopError::InvalidSseEvent)?;
        let envelope: serde_json::Value =
            serde_json::from_str(data).map_err(|_| DesktopError::InvalidSseEvent)?;
        let envelope_type = envelope
            .get("event_type")
            .and_then(serde_json::Value::as_str)
            .filter(|event_type| !event_type.is_empty())
            .ok_or(DesktopError::InvalidSseEvent)?;

        if event_type.is_some_and(|event_type| event_type != envelope_type) {
            return Err(DesktopError::InvalidSseEvent);
        }

        emit(StreamFrame::LiveEnvelope(envelope))?;
    }

    Ok(())
}

/// Owns a scoped desktop session without exposing stored bearer material through IPC.
pub struct DesktopSession<S> {
    lifecycle: Lifecycle<S>,
    cancelled_scopes: HashSet<String>,
}

/// Records the remote revocation result while guaranteeing local credential removal.
pub struct LogoutOutcome {
    pub remote_result: Result<(), DesktopError>,
    pub action: Option<LifecycleAction>,
}

impl<S: SecretStore> DesktopSession<S> {
    pub fn new(store: S) -> Self {
        Self {
            lifecycle: Lifecycle::new(store),
            cancelled_scopes: HashSet::new(),
        }
    }

    pub fn resume_with<T, F>(&mut self, scope: &SessionScope, execute: F) -> Result<T, DesktopError>
    where
        F: FnOnce(Request) -> Result<T, DesktopError>,
    {
        let result = self
            .lifecycle
            .store
            .load(scope)
            .map_err(|_| DesktopError::SessionInvalid)
            .and_then(|bearer| {
                build_authenticated_request(
                    scope.origin(),
                    "GET",
                    "/api/auth/me",
                    &bearer,
                    TransportKind::Rest,
                )
            })
            .and_then(execute);

        match result {
            Ok(value) => {
                self.lifecycle.transport_active = true;
                self.cancelled_scopes.remove(&scope.key());
                Ok(value)
            }
            Err(DesktopError::TransportUnavailable) => Err(DesktopError::TransportUnavailable),
            Err(error) => {
                self.expire(scope);
                Err(error)
            }
        }
    }

    pub fn store_session(
        &mut self,
        scope: &SessionScope,
        bearer: &str,
    ) -> Result<(), DesktopError> {
        match self.lifecycle.store_session(scope, bearer) {
            SessionState::Authenticated => {
                self.cancelled_scopes.remove(&scope.key());
                Ok(())
            }
            SessionState::Unauthenticated => Err(DesktopError::SessionInvalid),
        }
    }

    pub fn authenticated_request(
        &self,
        scope: &SessionScope,
        path: &str,
        transport: TransportKind,
    ) -> Result<Request, DesktopError> {
        self.authenticated_request_with_method(scope, "GET", path, transport)
    }

    pub fn authenticated_api_request(
        &self,
        scope: &SessionScope,
        request: DesktopApiRequest,
    ) -> Result<Request, DesktopError> {
        let bearer = self
            .lifecycle
            .store
            .load(scope)
            .map_err(|_| DesktopError::SessionInvalid)?;

        build_authenticated_api_request(scope.origin(), &bearer, request)
    }

    pub fn logout_with<F>(&mut self, scope: &SessionScope, execute: F) -> LogoutOutcome
    where
        F: FnOnce(Request) -> Result<(), DesktopError>,
    {
        let remote_result = self
            .authenticated_request_with_method(
                scope,
                "POST",
                "/api/auth/logout",
                TransportKind::Rest,
            )
            .and_then(execute);
        let action = self.revoke(scope);

        LogoutOutcome {
            remote_result,
            action,
        }
    }

    fn authenticated_request_with_method(
        &self,
        scope: &SessionScope,
        method: &str,
        path: &str,
        transport: TransportKind,
    ) -> Result<Request, DesktopError> {
        let bearer = self
            .lifecycle
            .store
            .load(scope)
            .map_err(|_| DesktopError::SessionInvalid)?;
        build_authenticated_request(scope.origin(), method, path, &bearer, transport)
    }

    pub fn connect_workspace_events<F>(
        &mut self,
        scope: &SessionScope,
        workspace: &str,
        execute: F,
    ) -> Result<WorkspaceEvent, DesktopError>
    where
        F: FnOnce(Request) -> Result<String, DesktopError>,
    {
        validate_workspace(workspace)?;
        let bearer = self
            .lifecycle
            .store
            .load(scope)
            .map_err(|_| DesktopError::SessionInvalid)?;
        let request = build_authenticated_request(
            scope.origin(),
            "GET",
            &format!("/api/workspaces/{workspace}/events"),
            &bearer,
            TransportKind::Sse,
        )?;
        let event = execute(request).and_then(|body| normalize_sse_event(&body, workspace));

        if let Err(error) = &event
            && *error != DesktopError::TransportUnavailable
        {
            self.expire(scope);
        }

        event
    }

    pub fn revoke(&mut self, scope: &SessionScope) -> Option<LifecycleAction> {
        self.expire(scope);
        self.lifecycle.take_action()
    }

    pub fn take_action(&mut self) -> Option<LifecycleAction> {
        self.lifecycle.take_action()
    }

    /// Best-effort second deletion used by Tauri's fail-closed cleanup path.
    pub fn remove_stored_session(&mut self, scope: &SessionScope) -> Result<(), &'static str> {
        match self.lifecycle.store.remove(scope) {
            Ok(()) | Err(SecretStoreError::Unavailable) => Ok(()),
        }
    }

    pub fn transport_is_cancelled(&self, scope: &SessionScope) -> bool {
        self.cancelled_scopes.contains(&scope.key())
    }

    fn expire(&mut self, scope: &SessionScope) {
        self.cancelled_scopes.insert(scope.key());
        self.lifecycle.expire_or_revoke(scope);
    }
}

fn normalize_sse_event(body: &str, workspace: &str) -> Result<WorkspaceEvent, DesktopError> {
    let data = body
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .ok_or(DesktopError::InvalidSseEvent)?;
    let envelope: serde_json::Value =
        serde_json::from_str(data).map_err(|_| DesktopError::InvalidSseEvent)?;
    let event_type = envelope
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .filter(|event_type| !event_type.is_empty())
        .ok_or(DesktopError::InvalidSseEvent)?;
    let workspace_id = envelope
        .get("workspace_id")
        .and_then(serde_json::Value::as_str)
        .filter(|workspace_id| !workspace_id.is_empty())
        .ok_or(DesktopError::InvalidSseEvent)?;
    let data = envelope
        .get("data")
        .cloned()
        .ok_or(DesktopError::InvalidSseEvent)?;

    if workspace_id.is_empty() || workspace.is_empty() {
        return Err(DesktopError::InvalidSseEvent);
    }

    Ok(WorkspaceEvent {
        event_type: event_type.to_owned(),
        data,
    })
}

fn validate_workspace(workspace: &str) -> Result<(), DesktopError> {
    if workspace.is_empty()
        || workspace
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err(DesktopError::InvalidWorkspace);
    }

    Ok(())
}

pub fn validate_workspace_slug(workspace: &str) -> Result<(), DesktopError> {
    validate_workspace(workspace)
}

fn canonical_origin(origin: &str) -> Result<String, DesktopError> {
    let origin = origin.strip_suffix('/').unwrap_or(origin);
    if origin != origin.trim() || origin != origin.to_ascii_lowercase() {
        return Err(DesktopError::InvalidOrigin);
    }

    let authority = origin
        .strip_prefix("https://")
        .ok_or(DesktopError::InvalidOrigin)?;
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '\\']) {
        return Err(DesktopError::InvalidOrigin);
    }

    let url = Url::parse(origin).map_err(|_| DesktopError::InvalidOrigin)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DesktopError::InvalidOrigin);
    }
    let host = url.host_str().ok_or(DesktopError::InvalidOrigin)?;
    let unbracketed_host = host.trim_start_matches('[').trim_end_matches(']');
    let canonical_host = if let Ok(address) = unbracketed_host.parse::<Ipv6Addr>() {
        format!("[{address}]")
    } else {
        unbracketed_host.to_owned()
    };
    let canonical = match url.port() {
        Some(443) | None => format!("https://{canonical_host}"),
        Some(port) => format!("https://{canonical_host}:{port}"),
    };
    if origin != canonical {
        return Err(DesktopError::InvalidOrigin);
    }

    if !canonical_host.starts_with('[')
        && canonical_host.split('.').count() == 4
        && authority
            .split('.')
            .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()))
        && canonical_host != host
    {
        return Err(DesktopError::InvalidOrigin);
    }

    if unbracketed_host.parse::<IpAddr>().is_err() {
        if unbracketed_host.split('.').count() == 4
            && unbracketed_host
                .split('.')
                .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(DesktopError::InvalidOrigin);
        }

        if unbracketed_host.len() > 253
            || unbracketed_host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
        {
            return Err(DesktopError::InvalidOrigin);
        }
    }

    Ok(canonical)
}

fn validate_api_path(path: &str) -> Result<(), DesktopError> {
    if !path.starts_with("/api/")
        || path.starts_with("//")
        || path.contains('\\')
        || path.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DesktopError::InvalidApiPath);
    }

    let path_only = path.split_once(['?', '#']).map_or(path, |(value, _)| value);
    for segment in path_only.split('/') {
        let decoded = percent_decode(segment)?;
        if decoded == "."
            || decoded == ".."
            || decoded.contains('\\')
            || decoded.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(DesktopError::InvalidApiPath);
        }
    }

    Ok(())
}

fn percent_decode(segment: &str) -> Result<String, DesktopError> {
    let mut bytes = segment.bytes();
    let mut decoded = Vec::with_capacity(segment.len());

    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }

        let high = bytes.next().ok_or(DesktopError::InvalidApiPath)?;
        let low = bytes.next().ok_or(DesktopError::InvalidApiPath)?;
        decoded.push((hex_value(high)? << 4) | hex_value(low)?);
    }

    String::from_utf8(decoded).map_err(|_| DesktopError::InvalidApiPath)
}

fn hex_value(byte: u8) -> Result<u8, DesktopError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DesktopError::InvalidApiPath),
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DesktopError {
    #[error("the desktop origin is invalid")]
    InvalidOrigin,
    #[error("the desktop identity is invalid")]
    InvalidIdentity,
    #[error("the desktop API path is invalid")]
    InvalidApiPath,
    #[error("the desktop HTTP method is invalid")]
    InvalidMethod,
    #[error("the bearer material is invalid")]
    InvalidBearer,
    #[error("the desktop HTTP header is invalid")]
    InvalidHeader,
    #[error("desktop transport is unavailable")]
    TransportUnavailable,
    #[error("the desktop session is invalid")]
    SessionInvalid,
    #[error("the desktop workspace is invalid")]
    InvalidWorkspace,
    #[error("the desktop SSE event is invalid")]
    InvalidSseEvent,
    #[error("desktop event delivery failed")]
    EventDelivery,
    #[error("desktop configuration is unavailable")]
    ConfigurationUnavailable,
}
