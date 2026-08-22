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
    time::Duration,
};
use thiserror::Error;
use zeroize::Zeroizing;

#[cfg(feature = "desktop-gate")]
pub mod gate;

const KEYRING_SERVICE: &str = "atlas-desktop";
const ACTIVE_IDENTITY_ACCOUNT_PREFIX: &str = "active-identity:";

pub trait TransportFactory {
    fn client(&self) -> Result<reqwest::Client, reqwest::Error>;
}

/// Bounds for the shared desktop HTTP client. A client-wide total timeout is
/// deliberately absent: the same client carries the long-lived workspace SSE
/// stream and attachment transfers through `desktop_api_request`, both of which
/// a global deadline would kill mid-flight. The read timeout still detects a
/// stalled connection — the server emits SSE keep-alives every 15 seconds —
/// without capping how long an actively progressing transfer may run. Requests
/// that need a hard total bound set one per request, as the logout flow does.
pub const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default)]
pub struct ReqwestTransportFactory;

impl ReqwestTransportFactory {
    pub fn system() -> Self {
        Self
    }
}

impl TransportFactory for ReqwestTransportFactory {
    fn client(&self) -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder()
            .connect_timeout(CLIENT_CONNECT_TIMEOUT)
            .read_timeout(CLIENT_READ_TIMEOUT)
            .build()
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

const DEFAULT_ZOOM_FACTOR: f64 = 1.0;
const MIN_ZOOM_FACTOR: f64 = 0.5;
const MAX_ZOOM_FACTOR: f64 = 3.0;

fn default_zoom_factor() -> f64 {
    DEFAULT_ZOOM_FACTOR
}

fn default_system_tray() -> bool {
    true
}

/// Normalizes a stored or requested zoom factor into the supported range, mapping any
/// non-finite value (NaN, infinities) back to the default rather than propagating it.
///
/// Rounds to two decimals so repeated additive stepping cannot accumulate binary
/// floating-point noise (for example `1.2000000000000002`) into the persisted value.
fn clamp_zoom(value: f64) -> f64 {
    if value.is_finite() {
        let clamped = value.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
        (clamped * 100.0).round() / 100.0
    } else {
        DEFAULT_ZOOM_FACTOR
    }
}

/// Machine-local desktop preferences, distinct from `DesktopConfiguration`. Stored in
/// `preferences.json`, a sibling of `desktop.json`, and never synced with the server.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct DesktopPreferences {
    window_decorations: bool,
    #[serde(default = "default_zoom_factor")]
    zoom_factor: f64,
    /// Defaulted so preferences written before start-on-login landed still load.
    #[serde(default)]
    start_on_login: bool,
    #[serde(default = "default_system_tray")]
    system_tray: bool,
    /// Absent until the window has been moved or resized at least once, so a
    /// fresh install still opens at the size `tauri.conf.json` asks for, and the
    /// stored file keeps the exact shape it had before this preference existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window_geometry: Option<WindowGeometry>,
}

/// The window's last position and size, in physical pixels.
///
/// Physical rather than logical because that is what Tauri reports and accepts
/// without a scale factor, and a stored geometry that has to be re-scaled by a
/// possibly different monitor is a geometry that drifts every restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default)]
    maximized: bool,
}

/// Smallest window worth restoring. A stored size below this is treated as
/// corrupt: restoring it would open a window the user cannot use or find.
const MIN_RESTORED_WINDOW_SIZE: u32 = 320;

impl WindowGeometry {
    pub fn new(x: i32, y: i32, width: u32, height: u32, maximized: bool) -> Self {
        Self {
            x,
            y,
            width,
            height,
            maximized,
        }
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn maximized(&self) -> bool {
        self.maximized
    }

    /// Whether this geometry is worth restoring at all.
    fn is_usable(&self) -> bool {
        self.width >= MIN_RESTORED_WINDOW_SIZE && self.height >= MIN_RESTORED_WINDOW_SIZE
    }

    /// Whether the window would land on one of `monitors`, each given as
    /// `(x, y, width, height)` in physical pixels.
    ///
    /// Checked against the window's top-left corner plus a margin, so a window
    /// stored on a monitor that is no longer attached — an undocked laptop, a
    /// display swapped for another — reopens where it can be seen instead of
    /// off-screen where it cannot be dragged back.
    pub fn is_visible_on(&self, monitors: &[(i32, i32, u32, u32)]) -> bool {
        const VISIBLE_MARGIN: i32 = 80;

        monitors.iter().any(|&(mx, my, mw, mh)| {
            let right = mx.saturating_add(i32::try_from(mw).unwrap_or(i32::MAX));
            let bottom = my.saturating_add(i32::try_from(mh).unwrap_or(i32::MAX));

            self.x + VISIBLE_MARGIN >= mx
                && self.y + VISIBLE_MARGIN >= my
                && self.x <= right.saturating_sub(VISIBLE_MARGIN)
                && self.y <= bottom.saturating_sub(VISIBLE_MARGIN)
        })
    }

    /// The geometry worth writing after a change.
    ///
    /// While the window is maximized its reported rectangle is the whole
    /// screen, so storing it would make the next restore-down fill the display
    /// and lose the size the user actually chose. The previously stored
    /// rectangle is kept in that case and only the maximized flag moves.
    pub fn to_store(previous: Option<Self>, current: Self) -> Self {
        match (current.maximized, previous) {
            (true, Some(previous)) if previous.is_usable() => Self {
                maximized: true,
                ..previous
            },
            _ => current,
        }
    }

    /// The geometry to restore on this set of monitors, or `None` when the
    /// stored one cannot be trusted.
    pub fn restorable_on(self, monitors: &[(i32, i32, u32, u32)]) -> Option<Self> {
        if !self.is_usable() {
            return None;
        }
        // An empty monitor list means the host could not enumerate displays;
        // trusting the stored position is better than discarding it over a
        // failed query.
        if !monitors.is_empty() && !self.is_visible_on(monitors) {
            return None;
        }
        Some(self)
    }
}

impl DesktopPreferences {
    const DECORATIONS_ON: Self = Self {
        window_decorations: true,
        zoom_factor: DEFAULT_ZOOM_FACTOR,
        start_on_login: false,
        system_tray: true,
        window_geometry: None,
    };

    /// Resolves stored preference bytes to the effective value, falling back to the safe
    /// default whenever storage is absent or does not parse. A stored zoom factor is
    /// normalized into the supported range so a corrupted value cannot reach the webview.
    pub fn resolve(stored: Option<&str>) -> Self {
        stored
            .and_then(|contents| serde_json::from_str::<Self>(contents).ok())
            .map(|preferences| Self {
                window_decorations: preferences.window_decorations,
                zoom_factor: clamp_zoom(preferences.zoom_factor),
                start_on_login: preferences.start_on_login,
                system_tray: preferences.system_tray,
                window_geometry: preferences.window_geometry,
            })
            .unwrap_or(Self::DECORATIONS_ON)
    }

    pub fn with_window_decorations(window_decorations: bool) -> Self {
        Self {
            window_decorations,
            zoom_factor: DEFAULT_ZOOM_FACTOR,
            start_on_login: false,
            system_tray: true,
            window_geometry: None,
        }
    }

    pub fn window_decorations(&self) -> bool {
        self.window_decorations
    }

    pub fn zoom_factor(&self) -> f64 {
        self.zoom_factor
    }

    pub fn start_on_login(&self) -> bool {
        self.start_on_login
    }

    pub fn system_tray(&self) -> bool {
        self.system_tray
    }

    /// Returns a copy with the start-on-login preference replaced, preserving the
    /// window and zoom preferences.
    pub fn set_start_on_login(self, start_on_login: bool) -> Self {
        Self {
            start_on_login,
            ..self
        }
    }

    pub fn window_geometry(&self) -> Option<WindowGeometry> {
        self.window_geometry
    }

    /// Returns a copy with the window geometry replaced, preserving every other
    /// preference.
    pub fn set_window_geometry(self, window_geometry: WindowGeometry) -> Self {
        Self {
            window_geometry: Some(window_geometry),
            ..self
        }
    }

    pub fn set_system_tray(self, system_tray: bool) -> Self {
        Self {
            system_tray,
            ..self
        }
    }

    /// Returns a copy with the zoom factor clamped into range, preserving the window
    /// decorations preference.
    pub fn set_zoom_factor(self, zoom_factor: f64) -> Self {
        Self {
            zoom_factor: clamp_zoom(zoom_factor),
            ..self
        }
    }

    /// Returns a copy with the window decorations preference replaced, preserving the
    /// stored zoom factor.
    pub fn set_window_decorations_value(self, window_decorations: bool) -> Self {
        Self {
            window_decorations,
            ..self
        }
    }

    pub fn load(directory: &Path) -> Self {
        let stored = fs::read_to_string(directory.join("preferences.json")).ok();
        Self::resolve(stored.as_deref())
    }

    pub fn save(&self, directory: &Path) -> Result<(), DesktopError> {
        fs::create_dir_all(directory).map_err(|_| DesktopError::ConfigurationUnavailable)?;
        let preferences =
            serde_json::to_string(self).map_err(|_| DesktopError::ConfigurationUnavailable)?;

        fs::write(
            directory.join("preferences.json"),
            format!("{preferences}\n"),
        )
        .map_err(|_| DesktopError::ConfigurationUnavailable)
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
    /// Loads the stored bearer.
    ///
    /// Returned in a `Zeroizing` wrapper so the copy this process makes of the
    /// token is wiped when the caller drops it, instead of lingering in freed
    /// heap memory for the rest of the session.
    fn load(&self, scope: &SessionScope) -> Result<Zeroizing<String>, SecretStoreError>;
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

    fn load(&self, scope: &SessionScope) -> Result<Zeroizing<String>, SecretStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &scope.key())
            .map_err(|_| SecretStoreError::Unavailable)?;
        entry
            .get_password()
            .map(Zeroizing::new)
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

    fn load(&self, scope: &SessionScope) -> Result<Zeroizing<String>, SecretStoreError> {
        if self.locked {
            return Err(SecretStoreError::Unavailable);
        }

        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable)?
            .get(&scope.key())
            .cloned()
            .map(Zeroizing::new)
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

/// How a workspace SSE stream ended, and therefore how the host must react.
///
/// The two terminal variants surface `atlas://workspace-closed` to the frontend
/// (which tears the source down); `Reconnect` is a benign end that the host
/// recovers from in-process, mirroring a native `EventSource`'s transparent
/// reconnect, so the frontend never sees a close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamTermination {
    /// A benign end — normal end-of-stream, keep-alive/idle recycle, network
    /// read error, connection reset, or a transient server error (5xx). The host
    /// reconnects the upstream instead of closing the frontend source.
    Reconnect,
    /// Authentication was lost (a `401`). The scoped session is revoked and the
    /// frontend source is closed — this must keep surfacing exactly as before.
    AuthLoss,
    /// A non-auth terminal condition: any other `4xx` (forbidden, workspace gone,
    /// bad request). The frontend source is closed, but the session is left intact
    /// so access to other workspaces is not revoked.
    Terminal,
}

/// Classifies a workspace SSE termination as the single source of truth for
/// "is this terminal?". A `401` is auth loss; any other `4xx` is a non-auth
/// terminal condition (forbidden, workspace gone); everything else — no status
/// (end-of-stream / read error) or a `5xx` — is a benign reconnect.
pub fn classify_workspace_stream_terminal(status: Option<u16>) -> StreamTermination {
    match status {
        Some(401) => StreamTermination::AuthLoss,
        Some(code) if (400..500).contains(&code) => StreamTermination::Terminal,
        _ => StreamTermination::Reconnect,
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

        // An SSE comment / keep-alive frame (axum sends `:\n\n` on an idle stream)
        // carries no `data:` field. Per the SSE spec a data-less event is never
        // dispatched, so it is ignored rather than treated as a protocol error —
        // otherwise every idle keep-alive would tear the stream down.
        let Some(data) = data else {
            continue;
        };
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

/// Upper bound on the remote logout revocation. A slow or unresponsive server
/// must never stall local credential teardown or the login redirect, so the
/// request is bounded and its failure is treated as best-effort.
pub const LOGOUT_REMOTE_TIMEOUT: Duration = Duration::from_secs(5);

fn with_logout_remote_timeout(mut request: Request) -> Request {
    *request.timeout_mut() = Some(LOGOUT_REMOTE_TIMEOUT);
    request
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
        let request = self.begin_resume(scope)?;

        self.complete_resume(scope, execute(request))
    }

    /// First half of the resume flow: loads the stored bearer and builds the
    /// identity probe request. Split from [`Self::complete_resume`] so the
    /// network execution can run on an async runtime without holding the
    /// session lock; a load or build failure expires the scope immediately.
    pub fn begin_resume(&mut self, scope: &SessionScope) -> Result<Request, DesktopError> {
        let request = self
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
            });

        match request {
            Ok(request) => Ok(request),
            Err(error) => {
                self.expire(scope);
                Err(error)
            }
        }
    }

    /// Second half of the resume flow: records the executed probe result. A
    /// success reactivates the scope; any failure other than an unavailable
    /// transport expires it.
    pub fn complete_resume<T>(
        &mut self,
        scope: &SessionScope,
        result: Result<T, DesktopError>,
    ) -> Result<T, DesktopError> {
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
            .map(with_logout_remote_timeout)
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

#[cfg(test)]
mod desktop_preferences_tests {
    use super::*;

    fn geometry(x: i32, y: i32) -> WindowGeometry {
        WindowGeometry::new(x, y, 1200, 800, false)
    }

    const LAPTOP: (i32, i32, u32, u32) = (0, 0, 1920, 1080);
    const SECOND_SCREEN: (i32, i32, u32, u32) = (1920, 0, 2560, 1440);

    #[test]
    fn a_geometry_on_an_attached_monitor_is_restored() {
        let stored = geometry(100, 100);

        assert_eq!(stored.restorable_on(&[LAPTOP, SECOND_SCREEN]), Some(stored),);
    }

    #[test]
    fn a_geometry_on_a_monitor_that_is_gone_is_dropped() {
        // Saved on the second screen, reopened with only the laptop attached:
        // restoring it would put the window where it cannot be dragged back.
        let stored = geometry(2400, 300);

        assert_eq!(stored.restorable_on(&[LAPTOP]), None);
        assert_eq!(stored.restorable_on(&[LAPTOP, SECOND_SCREEN]), Some(stored));
    }

    #[test]
    fn a_geometry_survives_a_failed_monitor_query() {
        let stored = geometry(2400, 300);

        assert_eq!(
            stored.restorable_on(&[]),
            Some(stored),
            "an empty monitor list means the host could not tell, not that the position is bad"
        );
    }

    #[test]
    fn a_window_too_small_to_use_is_not_restored() {
        let stored = WindowGeometry::new(100, 100, 20, 20, false);

        assert_eq!(stored.restorable_on(&[LAPTOP]), None);
    }

    #[test]
    fn maximizing_keeps_the_size_the_user_chose() {
        let chosen = geometry(200, 150);
        let maximized = WindowGeometry::new(0, 0, 1920, 1080, true);

        let stored = WindowGeometry::to_store(Some(chosen), maximized);

        assert_eq!(stored.width(), chosen.width());
        assert_eq!(stored.height(), chosen.height());
        assert_eq!(stored.x(), chosen.x());
        assert!(
            stored.maximized(),
            "the maximized state itself is remembered"
        );
    }

    #[test]
    fn maximizing_without_a_previous_size_stores_what_it_has() {
        let maximized = WindowGeometry::new(0, 0, 1920, 1080, true);

        assert_eq!(WindowGeometry::to_store(None, maximized), maximized);
    }

    #[test]
    fn an_ordinary_resize_replaces_the_stored_geometry() {
        let previous = geometry(200, 150);
        let resized = WindowGeometry::new(300, 250, 1000, 700, false);

        assert_eq!(WindowGeometry::to_store(Some(previous), resized), resized);
    }

    #[test]
    fn preferences_without_a_geometry_still_load() {
        let preferences = DesktopPreferences::resolve(Some(
            "{\"window_decorations\":true,\"zoom_factor\":1.0,\"start_on_login\":false,\"system_tray\":true}",
        ));

        assert_eq!(preferences.window_geometry(), None);
    }

    #[test]
    fn a_stored_geometry_round_trips_through_the_preference_file() {
        let stored = DesktopPreferences::DECORATIONS_ON.set_window_geometry(geometry(10, 20));
        let serialized = serde_json::to_string(&stored).unwrap_or_default();

        assert_eq!(
            DesktopPreferences::resolve(Some(&serialized)).window_geometry(),
            Some(geometry(10, 20))
        );
    }

    #[test]
    fn resolves_to_on_when_no_preference_is_stored() {
        let preferences = DesktopPreferences::resolve(None);

        assert_eq!(preferences, DesktopPreferences::DECORATIONS_ON);
        assert!(!preferences.start_on_login());
        assert!(preferences.system_tray());
    }

    #[test]
    fn resolves_to_on_when_the_stored_preference_does_not_parse() {
        for stored in [
            "not json",
            "{\"window_decorations\": \"nope\"}",
            "{}",
            "null",
            "",
        ] {
            assert_eq!(
                DesktopPreferences::resolve(Some(stored)),
                DesktopPreferences::DECORATIONS_ON,
                "{stored:?} must resolve to the safe default"
            );
        }
    }

    #[test]
    fn honors_a_stored_off_preference() {
        let preferences = DesktopPreferences::resolve(Some(
            "{\"window_decorations\":false,\"zoom_factor\":1.25,\"start_on_login\":true}",
        ));

        assert!(!preferences.window_decorations());
        assert_eq!(preferences.zoom_factor(), 1.25);
        assert!(preferences.start_on_login());
        assert!(preferences.system_tray());
    }

    #[test]
    fn resolves_a_legacy_preference_without_a_zoom_factor_to_the_default_zoom() {
        let resolved = DesktopPreferences::resolve(Some("{\"window_decorations\":false}"));

        assert!(!resolved.window_decorations());
        assert_eq!(resolved.zoom_factor(), DEFAULT_ZOOM_FACTOR);
    }

    #[test]
    fn honors_a_stored_zoom_factor_within_range() {
        let resolved =
            DesktopPreferences::resolve(Some("{\"window_decorations\":true,\"zoom_factor\":1.5}"));

        assert!(resolved.window_decorations());
        assert_eq!(resolved.zoom_factor(), 1.5);
    }

    #[test]
    fn clamps_an_out_of_range_or_non_finite_stored_zoom_factor() {
        assert_eq!(
            DesktopPreferences::resolve(Some("{\"window_decorations\":true,\"zoom_factor\":9.0}"))
                .zoom_factor(),
            MAX_ZOOM_FACTOR
        );
        assert_eq!(
            DesktopPreferences::resolve(Some("{\"window_decorations\":true,\"zoom_factor\":0.1}"))
                .zoom_factor(),
            MIN_ZOOM_FACTOR
        );
        assert_eq!(
            DesktopPreferences::resolve(Some("{\"window_decorations\":true,\"zoom_factor\":null}"))
                .zoom_factor(),
            DEFAULT_ZOOM_FACTOR
        );
    }

    #[test]
    fn builders_preserve_the_sibling_preference() {
        let zoomed = DesktopPreferences::with_window_decorations(false).set_zoom_factor(1.5);
        assert!(!zoomed.window_decorations());
        assert_eq!(zoomed.zoom_factor(), 1.5);

        let toggled = zoomed.set_window_decorations_value(true);
        assert!(toggled.window_decorations());
        assert_eq!(toggled.zoom_factor(), 1.5);
    }

    #[test]
    fn system_tray_setter_preserves_every_other_preference() {
        let preferences = DesktopPreferences::with_window_decorations(false)
            .set_zoom_factor(1.25)
            .set_start_on_login(true)
            .set_system_tray(false);

        assert!(!preferences.window_decorations());
        assert_eq!(preferences.zoom_factor(), 1.25);
        assert!(preferences.start_on_login());
        assert!(!preferences.system_tray());
    }

    #[test]
    fn set_zoom_factor_clamps_out_of_range_input() {
        assert_eq!(
            DesktopPreferences::with_window_decorations(true)
                .set_zoom_factor(9.0)
                .zoom_factor(),
            MAX_ZOOM_FACTOR
        );
        assert_eq!(
            DesktopPreferences::with_window_decorations(true)
                .set_zoom_factor(f64::NAN)
                .zoom_factor(),
            DEFAULT_ZOOM_FACTOR
        );
    }

    #[test]
    fn set_zoom_factor_rounds_accumulated_floating_point_noise() {
        assert_eq!(
            DesktopPreferences::with_window_decorations(true)
                .set_zoom_factor(1.2000000000000002)
                .zoom_factor(),
            1.2
        );
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExternalUrlError {
    #[error("the external URL is malformed")]
    Malformed,
    #[error("the external URL scheme is not allowed")]
    UnsupportedScheme,
    #[error("the external URL carries credentials")]
    CredentialsPresent,
}

/// Validates a URL the webview asked the host to open in the user's real browser.
///
/// The webview is the untrusted side of this boundary: it can ask the host to
/// open anything, and `opener` hands the string to whatever handler the
/// operating system registered for the scheme. Only `http` and `https` are
/// ever passed through, and embedded credentials are rejected so a crafted
/// link cannot smuggle a password into a system-level open call.
pub fn validate_external_url(raw: &str) -> Result<Url, ExternalUrlError> {
    let url = Url::parse(raw).map_err(|_| ExternalUrlError::Malformed)?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(ExternalUrlError::UnsupportedScheme);
    }

    if url.host_str().is_none() {
        return Err(ExternalUrlError::Malformed);
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(ExternalUrlError::CredentialsPresent);
    }

    Ok(url)
}

/// The window operations a second launch performs to surface the running instance.
/// Abstracted so the ordering and failure handling are testable without a runtime.
pub trait SurfaceableWindow {
    fn unminimize(&self) -> Result<(), String>;
    fn show(&self) -> Result<(), String>;
    fn set_focus(&self) -> Result<(), String>;
}

/// Brings the already-running window forward when a second launch is rejected.
///
/// Every step is attempted even after one fails: the window states are
/// independent (it can be hidden without being minimized, or focused while
/// hidden), so stopping at the first error can leave the user staring at a
/// desktop with no window despite having asked for one. The first error is
/// reported for logging once the window has been given every chance to appear.
pub fn surface_existing_window<W: SurfaceableWindow + ?Sized>(window: &W) -> Result<(), String> {
    let outcomes = [window.unminimize(), window.show(), window.set_focus()];

    outcomes
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), Err)
}

/// Reduces an attachment name to a single, safe file name for the downloads
/// directory.
///
/// The name is server data that any workspace member can choose, so it is treated
/// as untrusted: path separators, parent references and control characters are
/// stripped rather than escaped, leaving a name that cannot steer the write out of
/// the target directory. A name that carries no usable characters falls back to
/// `download`.
pub fn sanitize_download_file_name(raw: &str) -> String {
    const FALLBACK: &str = "download";

    let last_segment = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .replace(|character: char| character.is_control(), "");

    if last_segment.is_empty() || last_segment.chars().all(|character| character == '.') {
        return FALLBACK.to_owned();
    }

    last_segment
}

/// Picks a free path for `file_name` inside `directory`, numbering the name when
/// it is taken so a download never overwrites an existing file. `exists` is
/// injected so the numbering is testable without touching a filesystem.
pub fn unique_download_path(
    directory: &Path,
    file_name: &str,
    exists: impl Fn(&Path) -> bool,
) -> std::path::PathBuf {
    let candidate = directory.join(file_name);
    if !exists(&candidate) {
        return candidate;
    }

    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (file_name, String::new()),
    };

    (1..)
        .map(|index| directory.join(format!("{stem} ({index}){extension}")))
        .find(|candidate| !exists(candidate))
        .unwrap_or(candidate)
}

/// What closing the main window should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseBehavior {
    /// Keep the process alive behind the tray icon, which can restore the window.
    HideToTray,
    /// Let the close proceed and end the process.
    Exit,
}

/// Decides what a window close means.
///
/// Hiding is only safe while a tray icon exists to bring the window back: several
/// Linux desktops ship without a system tray, and hiding there would leave a
/// running process the user can neither see nor quit.
pub fn close_behavior(tray_available: bool) -> CloseBehavior {
    if tray_available {
        CloseBehavior::HideToTray
    } else {
        CloseBehavior::Exit
    }
}

/// The subscriber filter, honouring `RUST_LOG` and falling back to the level the
/// host has always used.
pub fn default_log_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,atlas_desktop=debug".into())
}

/// Days of rotated logs kept on disk. Enough to cover a report of "it broke over
/// the weekend" without letting a chatty install grow without bound.
pub const LOG_RETENTION_DAYS: usize = 7;

/// Resolves the XDG state directory for desktop logs. The environment values are
/// passed in rather than read here so the resolution is testable without mutating
/// the process environment.
pub fn desktop_state_directory_from(
    xdg_state_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    if let Some(directory) = xdg_state_home {
        return Some(std::path::PathBuf::from(directory).join("atlas-desktop"));
    }

    home.map(|home| {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("atlas-desktop")
    })
}

/// Starts writing logs to a rotating file in `directory`, returning the worker
/// guard that must stay alive for the life of the process.
///
/// A GUI launched from a desktop menu has no stderr, which left production
/// failures undiagnosable. The directory is created private to the user: log
/// lines carry request paths and workspace slugs, which are not secrets but are
/// nobody else's business on a shared machine.
///
/// Returns `None` when the directory cannot be prepared, leaving the process to
/// run with stderr logging only rather than refusing to start.
pub fn install_file_logging(
    directory: &Path,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    fs::create_dir_all(directory).ok()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
    }

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("atlas-desktop.log")
        .max_log_files(LOG_RETENTION_DAYS)
        .build(directory)
        .ok()?;

    let (writer, guard) = tracing_appender::non_blocking(appender);

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // The stderr layer is kept alongside the file: it costs nothing when the
    // process has no terminal and stays useful when run from a shell in dev.
    tracing_subscriber::registry()
        .with(default_log_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(writer)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .try_init()
        .ok();

    Some(guard)
}
