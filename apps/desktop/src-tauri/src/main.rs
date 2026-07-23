use atlas_desktop::{
    DesktopApiRequest, DesktopConfiguration, DesktopError, DesktopPreferences, DesktopSession,
    LifecycleAction, ReqwestTransportFactory, SecretServiceStore, SessionScope, StreamFrame,
    StreamTermination, TransportFactory, TransportKind, classify_workspace_stream_terminal,
    clear_active_identity, load_active_identity, process_workspace_sse_chunk,
    store_active_identity,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    process,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, Runtime, State};

/// Registry key for a running workspace stream: the session scope
/// (`origin:identity`) plus the workspace slug, so a late stop for one
/// workspace can never cancel a newer subscription for another.
type TransportKey = (String, String);

/// One live SSE task plus the generation that owns it.
///
/// Generations make stop/subscribe races safe: the webview may still deliver a
/// late `desktop_workspace_events_stop` for generation N after generation N+1
/// has already replaced it. Without the generation check that stop would abort
/// the newer stream and leave the SPA believing live updates are connected.
struct TransportEntry {
    generation: u64,
    handle: tauri::async_runtime::JoinHandle<()>,
}

struct DesktopState {
    origin: Mutex<String>,
    configuration_directory: PathBuf,
    client: reqwest::Client,
    session: Arc<Mutex<DesktopSession<SecretServiceStore>>>,
    transports: Arc<Mutex<HashMap<TransportKey, TransportEntry>>>,
    /// Monotonic id assigned to each `desktop_workspace_events_subscribe` call.
    transport_generation: Mutex<u64>,
}

const DEFAULT_DESKTOP_ORIGIN: &str = "https://atlas.iperez.dev";
const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Serialize)]
struct IpcSessionStatus {
    authenticated: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IpcAgentIdentity {
    id: String,
    name: String,
    scopes: Vec<String>,
}

/// The only authenticated identity payload allowed to cross the Tauri boundary.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IpcIdentity {
    principal_type: String,
    username: String,
    email: Option<String>,
    id: Option<String>,
    display_name: Option<String>,
    is_root: bool,
    is_system_admin: bool,
    #[serde(default)]
    agent: Option<IpcAgentIdentity>,
}

#[derive(Serialize)]
struct IpcEmpty {}

/// Returned by `desktop_workspace_events_subscribe` so a later stop can target
/// exactly this stream generation.
#[derive(Serialize)]
struct IpcWorkspaceStream {
    generation: u64,
}

/// Metadata half of a `desktop_api_request` response. The body no longer
/// travels as a JSON number array (one text token per byte, parsed twice on the
/// webview thread); instead the command returns a framed byte buffer
/// (`u32` little-endian meta length ++ meta JSON ++ raw body) as a
/// `tauri::ipc::Response`, and the adapter reconstructs the `Response` from it.
#[derive(Serialize)]
struct IpcHttpResponseMeta {
    status: u16,
    headers: Vec<(String, String)>,
}

fn frame_ipc_http_response(meta: &IpcHttpResponseMeta, body: &[u8]) -> Result<Vec<u8>, String> {
    let meta_json =
        serde_json::to_vec(meta).map_err(|_| "desktop response encoding failed".to_owned())?;
    let meta_length = u32::try_from(meta_json.len())
        .map_err(|_| "desktop response encoding failed".to_owned())?;

    let mut framed = Vec::with_capacity(4 + meta_json.len() + body.len());
    framed.extend_from_slice(&meta_length.to_le_bytes());
    framed.extend_from_slice(&meta_json);
    framed.extend_from_slice(body);

    Ok(framed)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct IpcSessionAction {
    origin: String,
    identity: Option<String>,
    cancel_transport: bool,
}

#[derive(Deserialize)]
struct LoginCredentials {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct IpcResult<T> {
    data: Option<T>,
    error: Option<serde_json::Value>,
}

impl<T> IpcResult<T> {
    fn data(data: T) -> Self {
        Self {
            data: Some(data),
            error: None,
        }
    }

    fn error(error: &'static str) -> Self {
        Self {
            data: None,
            error: Some(serde_json::Value::String(error.to_owned())),
        }
    }

    fn error_value(error: serde_json::Value) -> Self {
        Self {
            data: None,
            error: Some(error),
        }
    }
}

fn scope_transport_key(scope: &SessionScope) -> String {
    format!("{}:{}", scope.origin(), scope.identity())
}

fn cancel_transport(state: &DesktopState, scope: &SessionScope) -> Result<(), &'static str> {
    let scope_key = scope_transport_key(scope);
    let mut transports = state
        .transports
        .lock()
        .map_err(|_| "desktop transport state is unavailable")?;
    let existing = std::mem::take(&mut *transports);

    for (key, entry) in existing {
        if key.0 == scope_key {
            entry.handle.abort();
        } else {
            transports.insert(key, entry);
        }
    }

    Ok(())
}

/// Stops the workspace stream. When `generation` is `Some`, only aborts if that
/// generation is still the active entry — a late stop for a replaced stream is a
/// no-op. When `generation` is `None`, aborts whatever is currently registered
/// (logout / scope teardown paths).
fn cancel_workspace_transport(
    state: &DesktopState,
    scope: &SessionScope,
    workspace_slug: &str,
    generation: Option<u64>,
) -> Result<(), &'static str> {
    let key = (scope_transport_key(scope), workspace_slug.to_owned());
    let mut transports = state
        .transports
        .lock()
        .map_err(|_| "desktop transport state is unavailable")?;

    let should_abort = match (generation, transports.get(&key)) {
        (Some(expected), Some(entry)) => entry.generation == expected,
        (None, Some(_)) => true,
        (_, None) => false,
    };

    if should_abort && let Some(entry) = transports.remove(&key) {
        entry.handle.abort();
    }

    Ok(())
}

fn cancel_transports_for_origin(state: &DesktopState) -> Result<(), &'static str> {
    let prefix = format!("{}:", current_origin(state)?);
    let mut transports = state
        .transports
        .lock()
        .map_err(|_| "desktop transport state is unavailable")?;
    let existing = std::mem::take(&mut *transports);

    for (key, entry) in existing {
        if key.0.starts_with(&prefix) {
            entry.handle.abort();
        } else {
            transports.insert(key, entry);
        }
    }

    Ok(())
}

fn next_transport_generation(state: &DesktopState) -> Result<u64, &'static str> {
    let mut generation = state
        .transport_generation
        .lock()
        .map_err(|_| "desktop transport state is unavailable")?;
    *generation = generation.wrapping_add(1);
    Ok(*generation)
}

fn emit_action<R: Runtime>(
    app: &tauri::AppHandle<R>,
    action: LifecycleAction,
) -> Result<(), &'static str> {
    emit_session_action(
        app,
        IpcSessionAction {
            origin: action.scope().origin().to_owned(),
            identity: Some(action.scope().identity().to_owned()),
            cancel_transport: action.cancels_transport(),
        },
    )
}

fn emit_session_action<R: Runtime>(
    app: &tauri::AppHandle<R>,
    action: IpcSessionAction,
) -> Result<(), &'static str> {
    app.emit("atlas://session-action", action)
        .map_err(|_| "desktop session action delivery failed")
}

fn fail_closed_origin<F>(state: &DesktopState, emit: F) -> Result<(), &'static str>
where
    F: FnOnce(IpcSessionAction) -> Result<(), &'static str>,
{
    cancel_transports_for_origin(state)?;
    emit(IpcSessionAction {
        origin: current_origin(state)?,
        identity: None,
        cancel_transport: true,
    })
}

fn current_origin(state: &DesktopState) -> Result<String, &'static str> {
    state
        .origin
        .lock()
        .map_err(|_| "desktop configuration state is unavailable")
        .map(|origin| origin.clone())
}

fn run_failed_resume_cleanup<C, D, A, I>(
    mut cancel: C,
    mut delete_secret: D,
    mut emit_scoped_action: A,
    mut clear_identity: I,
) -> Result<(), &'static str>
where
    C: FnMut() -> Result<(), &'static str>,
    D: FnMut() -> Result<(), &'static str>,
    A: FnMut() -> Result<(), &'static str>,
    I: FnMut() -> Result<(), &'static str>,
{
    let failures = [
        cancel(),
        delete_secret(),
        emit_scoped_action(),
        clear_identity(),
    ];

    if failures.iter().any(Result::is_err) {
        return Err("desktop session cleanup failed");
    }

    Ok(())
}

fn run_failed_resume_cleanup_for_scope<D, A, I>(
    state: &DesktopState,
    scope: &SessionScope,
    delete_secret: D,
    emit_scoped_action: A,
    clear_identity: I,
) -> Result<(), &'static str>
where
    D: FnMut() -> Result<(), &'static str>,
    A: FnMut() -> Result<(), &'static str>,
    I: FnMut() -> Result<(), &'static str>,
{
    run_failed_resume_cleanup(
        || cancel_transport(state, scope),
        delete_secret,
        emit_scoped_action,
        clear_identity,
    )
}

fn scope_for_active_identity_or_fail_closed_with<F, E>(
    state: &DesktopState,
    load_identity: F,
    emit: E,
) -> Result<SessionScope, &'static str>
where
    F: FnOnce(&str) -> Result<String, &'static str>,
    E: FnOnce(IpcSessionAction) -> Result<(), &'static str>,
{
    let origin = current_origin(state)?;
    let identity = match load_identity(&origin) {
        Ok(identity) => identity,
        Err(error) => {
            fail_closed_origin(state, emit)?;
            return Err(error);
        }
    };

    SessionScope::new(&origin, &identity).or_else(|_| {
        fail_closed_origin(state, emit)?;
        Err("desktop session is invalid")
    })
}

fn scope_for_active_identity_or_fail_closed<R: Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
) -> Result<SessionScope, &'static str> {
    scope_for_active_identity_or_fail_closed_with(
        state,
        |origin| load_active_identity(origin).map_err(|_| "desktop session is unavailable"),
        |action| emit_session_action(app, action),
    )
}

fn emit_workspace_closed<R: Runtime>(
    app: &tauri::AppHandle<R>,
    workspace_slug: &str,
) -> Result<(), &'static str> {
    app.emit(
        "atlas://workspace-closed",
        serde_json::json!({"workspace_slug": workspace_slug}),
    )
    .map_err(|_| "desktop workspace closure delivery failed")
}

fn emit_workspace_resync<R: Runtime>(
    app: &tauri::AppHandle<R>,
    workspace_slug: &str,
) -> Result<(), DesktopError> {
    app.emit(
        "atlas://workspace-resync",
        serde_json::json!({"workspace_slug": workspace_slug}),
    )
    .map_err(|_| DesktopError::EventDelivery)
}

fn emit_workspace_frame<R: Runtime>(
    app: &tauri::AppHandle<R>,
    workspace_slug: &str,
    frame: StreamFrame,
) -> Result<(), DesktopError> {
    match frame {
        StreamFrame::LiveEnvelope(envelope) => app
            .emit("atlas://workspace-event", envelope)
            .map_err(|_| DesktopError::EventDelivery),
        StreamFrame::Resync => emit_workspace_resync(app, workspace_slug),
    }
}

fn invalidate_scope<R: Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
    scope: &SessionScope,
) -> Result<(), &'static str> {
    cancel_transport(state, scope)?;

    let action = state
        .session
        .lock()
        .map_err(|_| "desktop session state is unavailable")?
        .revoke(scope);

    if let Some(action) = action {
        emit_action(app, action)?;
    }

    Ok(())
}

fn finalize_failed_resume<R: Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
    scope: &SessionScope,
) -> Result<(), &'static str> {
    let action = state
        .session
        .lock()
        .map_err(|_| "desktop session state is unavailable")?
        .take_action();

    run_failed_resume_cleanup_for_scope(
        state,
        scope,
        || {
            state
                .session
                .lock()
                .map_err(|_| "desktop session state is unavailable")?
                .remove_stored_session(scope)
        },
        || match action.as_ref() {
            Some(action) => emit_action(app, action.clone()),
            None => Err("desktop session cleanup failed"),
        },
        || clear_active_identity(scope.origin()).map_err(|_| "desktop session cleanup failed"),
    )
}

fn parse_desktop_identity(value: serde_json::Value) -> Result<IpcIdentity, &'static str> {
    serde_json::from_value(value).map_err(|_| "desktop session is invalid")
}

#[cfg(test)]
#[tauri::command]
fn desktop_test_identity(identity: serde_json::Value) -> IpcResult<IpcIdentity> {
    match parse_desktop_identity(identity) {
        Ok(identity) => IpcResult::data(identity),
        Err(error) => IpcResult::error(error),
    }
}

#[tauri::command]
fn desktop_session_status<R: Runtime>(
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> Result<IpcSessionStatus, String> {
    let session = state
        .session
        .lock()
        .map_err(|_| "desktop session state is unavailable".to_owned())?;

    Ok(IpcSessionStatus {
        authenticated: !session.transport_is_cancelled(
            &scope_for_active_identity_or_fail_closed(&state, &app).map_err(str::to_owned)?,
        ),
    })
}

#[tauri::command]
fn desktop_get_origin(state: State<'_, DesktopState>) -> IpcResult<DesktopConfiguration> {
    match current_origin(&state).and_then(|origin| {
        DesktopConfiguration::from_selected_origin(&origin)
            .map_err(|_| "desktop configuration is unavailable")
    }) {
        Ok(configuration) => IpcResult::data(configuration),
        Err(error) => IpcResult::error(error),
    }
}

#[tauri::command]
fn desktop_set_origin<R: Runtime>(
    origin: String,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> IpcResult<DesktopConfiguration> {
    let configuration = match DesktopConfiguration::from_selected_origin(&origin) {
        Ok(configuration) => configuration,
        Err(_) => return IpcResult::error("Enter a valid HTTPS Atlas server origin"),
    };
    let previous = match current_origin(&state) {
        Ok(previous) => previous,
        Err(error) => return IpcResult::error(error),
    };
    if previous != configuration.origin() {
        let cleanup_result = load_active_identity(&previous)
            .ok()
            .and_then(|identity| SessionScope::new(&previous, &identity).ok())
            .map(|scope| {
                cancel_transport(&state, &scope)?;
                let action = state
                    .session
                    .lock()
                    .map_err(|_| "desktop session state is unavailable")?
                    .revoke(&scope)
                    .ok_or("desktop session cleanup failed")?;
                emit_action(&app, action)?;
                clear_active_identity(&previous).map_err(|_| "desktop session cleanup failed")
            })
            .unwrap_or_else(|| {
                cancel_transports_for_origin(&state)?;
                emit_session_action(
                    &app,
                    IpcSessionAction {
                        origin: previous.clone(),
                        identity: None,
                        cancel_transport: true,
                    },
                )
            });
        if cleanup_result.is_err() {
            return IpcResult::error("desktop session cleanup failed");
        }

        if configuration.save(&state.configuration_directory).is_err() {
            return IpcResult::error("desktop configuration is unavailable");
        }

        match state.origin.lock() {
            Ok(mut selected) => *selected = configuration.origin().to_owned(),
            Err(_) => return IpcResult::error("desktop configuration state is unavailable"),
        }
    } else if configuration.save(&state.configuration_directory).is_err() {
        return IpcResult::error("desktop configuration is unavailable");
    }

    IpcResult::data(configuration)
}

fn stored_desktop_preferences(state: &DesktopState) -> IpcResult<DesktopPreferences> {
    IpcResult::data(DesktopPreferences::load(&state.configuration_directory))
}

#[tauri::command]
fn desktop_get_window_decorations(state: State<'_, DesktopState>) -> IpcResult<DesktopPreferences> {
    stored_desktop_preferences(&state)
}

#[tauri::command]
fn desktop_set_window_decorations<R: Runtime>(
    decorations: bool,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> IpcResult<DesktopPreferences> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return IpcResult::error("desktop window is unavailable");
    };
    if window.set_decorations(decorations).is_err() {
        return IpcResult::error("desktop window decorations are unavailable");
    }

    let preferences = DesktopPreferences::load(&state.configuration_directory)
        .set_window_decorations_value(decorations);
    if preferences.save(&state.configuration_directory).is_err() {
        return IpcResult::error("desktop configuration is unavailable");
    }

    IpcResult::data(preferences)
}

#[tauri::command]
fn desktop_get_zoom(state: State<'_, DesktopState>) -> IpcResult<DesktopPreferences> {
    stored_desktop_preferences(&state)
}

#[tauri::command]
fn desktop_set_zoom<R: Runtime>(
    zoom_factor: f64,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> IpcResult<DesktopPreferences> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return IpcResult::error("desktop window is unavailable");
    };

    let preferences =
        DesktopPreferences::load(&state.configuration_directory).set_zoom_factor(zoom_factor);
    if window.set_zoom(preferences.zoom_factor()).is_err() {
        return IpcResult::error("desktop window zoom is unavailable");
    }

    if preferences.save(&state.configuration_directory).is_err() {
        return IpcResult::error("desktop configuration is unavailable");
    }

    IpcResult::data(preferences)
}

/// Async so the three sequential network calls run on the Tauri async runtime
/// instead of blocking the GTK main thread. The state is pulled from the app
/// handle because an async command taking `State<'_, _>` would have to change
/// its wire shape to a `Result`.
#[tauri::command]
async fn desktop_auth_login<R: Runtime>(
    credentials: LoginCredentials,
    app: tauri::AppHandle<R>,
) -> IpcResult<IpcEmpty> {
    if credentials.username.is_empty() || credentials.password.is_empty() {
        return IpcResult::error("desktop authentication failed");
    }
    let Some(state) = app.try_state::<DesktopState>() else {
        return IpcResult::error("desktop session state is unavailable");
    };
    let origin = match current_origin(&state) {
        Ok(origin) => origin,
        Err(error) => return IpcResult::error(error),
    };
    let response = match state
        .client
        .post(format!("{origin}/api/auth/login"))
        .json(&serde_json::json!({"username": credentials.username, "password": credentials.password}))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return IpcResult::error("desktop authentication failed"),
    };
    if !response.status().is_success() {
        return match response.json::<serde_json::Value>().await {
            Ok(problem) => IpcResult::error_value(problem),
            Err(_) => IpcResult::error("desktop authentication failed"),
        };
    }
    let body: serde_json::Value = match response.json().await {
        Ok(body) => body,
        Err(_) => return IpcResult::error("desktop authentication failed"),
    };
    let (bearer, identity) = match (
        body.get("token").and_then(serde_json::Value::as_str),
        body.pointer("/user/id").and_then(serde_json::Value::as_str),
    ) {
        (Some(bearer), Some(identity)) => (bearer, identity),
        _ => return IpcResult::error("desktop authentication failed"),
    };
    let scope = match SessionScope::new(&origin, identity) {
        Ok(scope) => scope,
        Err(_) => return IpcResult::error("desktop session is invalid"),
    };
    let stored = state
        .session
        .lock()
        .ok()
        .and_then(|mut session| session.store_session(&scope, bearer).ok());
    if stored.is_none() {
        if invalidate_scope(&state, &app, &scope).is_err() {
            return IpcResult::error("desktop session action delivery failed");
        }
        return IpcResult::error("desktop session storage is unavailable");
    }
    if store_active_identity(&origin, identity).is_err() {
        if invalidate_scope(&state, &app, &scope).is_err() {
            return IpcResult::error("desktop session action delivery failed");
        }
        return IpcResult::error("desktop session storage is unavailable");
    }
    let request = match state.session.lock().ok().and_then(|session| {
        session
            .authenticated_request(&scope, "/api/auth/me", TransportKind::Rest)
            .ok()
    }) {
        Some(request) => request,
        None => {
            if invalidate_scope(&state, &app, &scope).is_err() {
                return IpcResult::error("desktop session action delivery failed");
            }
            return IpcResult::error("desktop session is unavailable");
        }
    };
    match state.client.execute(request).await {
        Ok(response) if response.status().is_success() => IpcResult::data(IpcEmpty {}),
        _ => {
            if invalidate_scope(&state, &app, &scope).is_err() {
                return IpcResult::error("desktop session action delivery failed");
            }
            IpcResult::error("desktop session validation failed")
        }
    }
}

async fn execute_identity_probe(
    client: &reqwest::Client,
    request: reqwest::Request,
) -> Result<IpcIdentity, DesktopError> {
    let response = client
        .execute(request)
        .await
        .map_err(|_| DesktopError::TransportUnavailable)?;
    if !response.status().is_success() {
        return Err(DesktopError::SessionInvalid);
    }

    let body = response
        .json()
        .await
        .map_err(|_| DesktopError::SessionInvalid)?;

    parse_desktop_identity(body).map_err(|_| DesktopError::SessionInvalid)
}

/// Async so the identity probe runs on the Tauri async runtime instead of
/// blocking the GTK main thread. The resume flow is split in two session
/// calls so the lock is never held across the network await.
async fn resume_desktop_authentication<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> IpcResult<IpcIdentity> {
    let Some(state) = app.try_state::<DesktopState>() else {
        return IpcResult::error("desktop session state is unavailable");
    };
    let scope = match scope_for_active_identity_or_fail_closed(&state, &app) {
        Ok(scope) => scope,
        Err(error) => return IpcResult::error(error),
    };
    let request = match state.session.lock() {
        Ok(mut session) => session.begin_resume(&scope),
        Err(_) => return IpcResult::error("desktop session state is unavailable"),
    };

    let result = match request {
        Ok(request) => {
            let probed = execute_identity_probe(&state.client, request).await;
            match state.session.lock() {
                Ok(mut session) => session.complete_resume(&scope, probed),
                Err(_) => return IpcResult::error("desktop session state is unavailable"),
            }
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(body) => IpcResult::data(body),
        Err(_) => match finalize_failed_resume(&state, &app, &scope) {
            Ok(()) => IpcResult::error("desktop session is invalid"),
            Err(error) => IpcResult::error(error),
        },
    }
}

#[tauri::command]
async fn desktop_auth_resume<R: Runtime>(app: tauri::AppHandle<R>) -> IpcResult<IpcIdentity> {
    resume_desktop_authentication(app).await
}

#[tauri::command]
async fn desktop_auth_me<R: Runtime>(app: tauri::AppHandle<R>) -> IpcResult<IpcIdentity> {
    resume_desktop_authentication(app).await
}

#[tauri::command]
fn desktop_auth_logout<R: Runtime>(
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> IpcResult<IpcEmpty> {
    let scope = match scope_for_active_identity_or_fail_closed(&state, &app) {
        Ok(scope) => scope,
        Err(error) => return IpcResult::error(error),
    };
    let outcome = match state.session.lock() {
        Ok(mut session) => session.logout_with(&scope, |request| {
            // The request carries a per-request timeout, which arms a Tokio timer at
            // execution. A sync command runs on the GTK main thread with no ambient
            // runtime, so the timed request must execute on Tauri's async runtime
            // (via spawn) rather than the calling thread, or arming the timer aborts.
            let client = state.client.clone();
            let execution =
                tauri::async_runtime::spawn(async move { client.execute(request).await });
            match tauri::async_runtime::block_on(execution) {
                Ok(Ok(response)) if response.status().is_success() => Ok(()),
                Ok(Ok(_)) => Err(DesktopError::SessionInvalid),
                Ok(Err(_)) | Err(_) => Err(DesktopError::TransportUnavailable),
            }
        }),
        Err(_) => return IpcResult::error("desktop session state is unavailable"),
    };

    let cancel_result = cancel_transport(&state, &scope);
    let action_result = outcome
        .action
        .map(|action| emit_action(&app, action))
        .unwrap_or(Err("desktop session action delivery failed"));
    let identity_result = clear_active_identity(scope.origin());

    if cancel_result.is_err() || action_result.is_err() || identity_result.is_err() {
        return IpcResult::error("desktop session cleanup failed");
    }

    match outcome.remote_result {
        Ok(()) => IpcResult::data(IpcEmpty {}),
        Err(_) => IpcResult::error("desktop session revocation failed"),
    }
}

#[tauri::command]
async fn desktop_api_request<R: Runtime>(
    request: DesktopApiRequest,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle<R>,
) -> Result<tauri::ipc::Response, String> {
    let scope = scope_for_active_identity_or_fail_closed(&state, &app).map_err(str::to_owned)?;
    let request = state
        .session
        .lock()
        .map_err(|_| "desktop session state is unavailable".to_owned())?
        .authenticated_api_request(&scope, request)
        .map_err(|error| error.to_string())?;
    let response = state
        .client
        .execute(request)
        .await
        .map_err(|_| "desktop transport is unavailable".to_owned())?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|_| "desktop transport is unavailable".to_owned())?
        .to_vec();

    if status == reqwest::StatusCode::UNAUTHORIZED {
        invalidate_scope(&state, &app, &scope).map_err(str::to_owned)?;
        clear_active_identity(scope.origin())
            .map_err(|_| "desktop session cleanup failed".to_owned())?;
    }

    let meta = IpcHttpResponseMeta {
        status: status.as_u16(),
        headers,
    };
    let framed = frame_ipc_http_response(&meta, &body)?;

    Ok(tauri::ipc::Response::new(framed))
}

/// Reads an image off the native clipboard and returns it as PNG, framed like
/// `desktop_api_request` (200 with the bytes, or 204 when the clipboard holds no
/// image). WebKitGTK does not surface pasted bitmaps through the webview's
/// `ClipboardEvent`, so the paste-to-attach flow falls back to this host command.
/// It runs synchronously on the GTK main thread, which is where the GDK clipboard
/// must be touched.
#[cfg(target_os = "linux")]
#[tauri::command]
fn desktop_read_clipboard_image() -> Result<tauri::ipc::Response, String> {
    let display = gtk::gdk::Display::default()
        .ok_or_else(|| "the desktop clipboard is unavailable".to_owned())?;
    let clipboard = gtk::Clipboard::default(&display)
        .ok_or_else(|| "the desktop clipboard is unavailable".to_owned())?;

    let Some(pixbuf) = clipboard.wait_for_image() else {
        let meta = IpcHttpResponseMeta {
            status: 204,
            headers: Vec::new(),
        };
        return Ok(tauri::ipc::Response::new(frame_ipc_http_response(
            &meta,
            &[],
        )?));
    };

    let png = pixbuf
        .save_to_bufferv("png", &[])
        .map_err(|_| "the clipboard image could not be encoded".to_owned())?;

    let meta = IpcHttpResponseMeta {
        status: 200,
        headers: vec![("content-type".to_owned(), "image/png".to_owned())],
    };
    Ok(tauri::ipc::Response::new(frame_ipc_http_response(
        &meta, &png,
    )?))
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn desktop_read_clipboard_image() -> Result<tauri::ipc::Response, String> {
    Err("clipboard image reading is not supported on this platform".to_owned())
}

#[tauri::command]
fn desktop_workspace_events_stop(
    workspace_slug: String,
    generation: Option<u64>,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle,
) -> IpcResult<IpcEmpty> {
    if atlas_desktop::validate_workspace_slug(&workspace_slug).is_err() {
        return IpcResult::error("desktop workspace is invalid");
    }
    let scope = match scope_for_active_identity_or_fail_closed(&state, &app) {
        Ok(scope) => scope,
        Err(error) => return IpcResult::error(error),
    };
    match cancel_workspace_transport(&state, &scope, &workspace_slug, generation) {
        Ok(()) => IpcResult::data(IpcEmpty {}),
        Err(error) => IpcResult::error(error),
    }
}

#[tauri::command]
fn desktop_workspace_events_subscribe(
    workspace_slug: String,
    state: State<'_, DesktopState>,
    app: tauri::AppHandle,
) -> IpcResult<IpcWorkspaceStream> {
    let scope = match scope_for_active_identity_or_fail_closed(&state, &app) {
        Ok(scope) => scope,
        Err(error) => return IpcResult::error(error),
    };
    let path = format!("/api/workspaces/{workspace_slug}/events");
    let request = match state.session.lock().ok().and_then(|session| {
        session
            .authenticated_request(&scope, &path, TransportKind::Sse)
            .ok()
    }) {
        Some(request) => request,
        None => {
            if invalidate_scope(&state, &app, &scope).is_err() {
                return IpcResult::error("desktop session action delivery failed");
            }
            return IpcResult::error("desktop session is unavailable");
        }
    };
    let generation = match next_transport_generation(&state) {
        Ok(generation) => generation,
        Err(error) => return IpcResult::error(error),
    };
    let key = (scope_transport_key(&scope), workspace_slug.clone());
    let session = Arc::clone(&state.session);
    let client = state.client.clone();
    let app_handle = app.clone();
    let scope_for_task = scope.clone();
    let handle = tauri::async_runtime::spawn(async move {
        run_workspace_stream(
            client,
            session,
            app_handle,
            scope_for_task,
            workspace_slug,
            path,
            request,
        )
        .await;
    });
    match state.transports.lock() {
        Ok(mut transports) => {
            if let Some(previous) = transports.insert(key, TransportEntry { generation, handle }) {
                previous.handle.abort();
            }
            IpcResult::data(IpcWorkspaceStream { generation })
        }
        Err(_) => {
            handle.abort();
            IpcResult::error("desktop transport state is unavailable")
        }
    }
}

/// Backoff floor for reconnecting a benignly-ended workspace SSE stream. A
/// healthy stream that recycles reconnects this fast; the ceiling below caps a
/// server that keeps ending the stream so the host never hammers it.
const WORKSPACE_STREAM_RECONNECT_INITIAL: Duration = Duration::from_secs(1);

/// Backoff ceiling for the workspace SSE reconnect loop.
const WORKSPACE_STREAM_RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Minimum lifetime for a connection to count as healthy and reset the backoff.
/// A connection that opens and ends sooner than this is treated as a failed
/// attempt, so a server that accepts then immediately closes still backs off
/// exponentially instead of reconnecting at the floor forever.
const WORKSPACE_STREAM_HEALTHY_MIN: Duration = Duration::from_secs(10);

/// The result of a single upstream SSE connection attempt.
struct WorkspaceStreamAttempt {
    /// How the attempt ended, and therefore how the loop must react.
    termination: StreamTermination,
    /// Whether the upstream connected and stayed up long enough to be healthy;
    /// a healthy attempt resets the reconnect backoff to its floor.
    healthy: bool,
}

/// Proxies a workspace SSE stream, transparently reconnecting on benign ends so
/// the frontend source mirrors a native `EventSource` and never sees a close for
/// a recoverable gap.
///
/// The loop terminates — emitting `atlas://workspace-closed` — only on a truly
/// terminal condition (auth loss, another `4xx`, or a vanished session). Every
/// other end (EOF, keep-alive recycle, read error, `5xx`) reconnects with bounded
/// exponential backoff. An explicit unsubscribe / stop aborts the spawned task at
/// its next await point (the `execute`/`sleep` calls), so no reconnect survives a
/// teardown and the stream registry stays the single lifecycle authority.
async fn run_workspace_stream<R: Runtime>(
    client: reqwest::Client,
    session: Arc<Mutex<DesktopSession<SecretServiceStore>>>,
    app: tauri::AppHandle<R>,
    scope: SessionScope,
    workspace_slug: String,
    path: String,
    initial_request: reqwest::Request,
) {
    let mut next_request = Some(initial_request);
    let mut backoff = WORKSPACE_STREAM_RECONNECT_INITIAL;
    let mut is_reconnect = false;

    loop {
        let request = match next_request.take() {
            Some(request) => request,
            None => match build_workspace_stream_request(&session, &scope, &path) {
                Some(request) => request,
                None => {
                    revoke_and_close(&session, &app, &scope, &workspace_slug);
                    return;
                }
            },
        };

        let attempt =
            run_workspace_stream_connection(&client, request, &app, &workspace_slug, is_reconnect)
                .await;

        match attempt.termination {
            StreamTermination::Reconnect => {
                if attempt.healthy {
                    backoff = WORKSPACE_STREAM_RECONNECT_INITIAL;
                }

                tokio::time::sleep(backoff).await;

                if !attempt.healthy {
                    backoff = (backoff * 2).min(WORKSPACE_STREAM_RECONNECT_MAX);
                }

                is_reconnect = true;
            }
            StreamTermination::AuthLoss => {
                revoke_and_close(&session, &app, &scope, &workspace_slug);
                return;
            }
            StreamTermination::Terminal => {
                let _ = emit_workspace_closed(&app, &workspace_slug);
                return;
            }
        }
    }
}

/// Runs one upstream SSE connection to completion, forwarding every live frame.
///
/// On a reconnect (`is_reconnect`) a single `atlas://workspace-resync` is emitted
/// once the upstream is open, so the frontend performs an atomic catch-up for any
/// events missed during the gap. This is the fallback for true transparent replay:
/// the Atlas SSE server assigns no event `id:` and honors no `Last-Event-ID`
/// header (its events come from an in-process broadcast with no replay log), so a
/// native `EventSource`'s silent replay cannot be reproduced.
async fn run_workspace_stream_connection<R: Runtime>(
    client: &reqwest::Client,
    request: reqwest::Request,
    app: &tauri::AppHandle<R>,
    workspace_slug: &str,
    is_reconnect: bool,
) -> WorkspaceStreamAttempt {
    let Ok(response) = client.execute(request).await else {
        return WorkspaceStreamAttempt {
            termination: StreamTermination::Reconnect,
            healthy: false,
        };
    };

    if !response.status().is_success() {
        return WorkspaceStreamAttempt {
            termination: classify_workspace_stream_terminal(Some(response.status().as_u16())),
            healthy: false,
        };
    }

    if is_reconnect && emit_workspace_resync(app, workspace_slug).is_err() {
        return WorkspaceStreamAttempt {
            termination: StreamTermination::Terminal,
            healthy: false,
        };
    }

    let opened_at = Instant::now();
    let mut stream = response.bytes_stream();
    let mut pending = String::new();

    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };

        if process_workspace_sse_chunk(&mut pending, &chunk, |frame| {
            emit_workspace_frame(app, workspace_slug, frame)
        })
        .is_err()
        {
            break;
        }
    }

    WorkspaceStreamAttempt {
        termination: StreamTermination::Reconnect,
        healthy: opened_at.elapsed() >= WORKSPACE_STREAM_HEALTHY_MIN,
    }
}

/// Rebuilds the authenticated upstream SSE request from the current session, so a
/// reconnect picks up a rotated bearer. `None` means the stored session is gone,
/// which the caller treats as terminal auth loss.
fn build_workspace_stream_request(
    session: &Arc<Mutex<DesktopSession<SecretServiceStore>>>,
    scope: &SessionScope,
    path: &str,
) -> Option<reqwest::Request> {
    session.lock().ok().and_then(|session| {
        session
            .authenticated_request(scope, path, TransportKind::Sse)
            .ok()
    })
}

/// Revokes the scoped session and closes the frontend source. Used for every
/// terminal auth-loss path so it keeps surfacing exactly as before.
fn revoke_and_close<R: Runtime>(
    session: &Arc<Mutex<DesktopSession<SecretServiceStore>>>,
    app: &tauri::AppHandle<R>,
    scope: &SessionScope,
    workspace_slug: &str,
) {
    let _ = revoke_from_transport(session, app, scope);
    let _ = emit_workspace_closed(app, workspace_slug);
}

fn revoke_from_transport<R: Runtime>(
    session: &Arc<Mutex<DesktopSession<SecretServiceStore>>>,
    app: &tauri::AppHandle<R>,
    scope: &SessionScope,
) -> Result<(), &'static str> {
    let action = session
        .lock()
        .map_err(|_| "desktop session state is unavailable")?
        .revoke(scope);
    if let Some(action) = action {
        emit_action(app, action)?;
    }
    Ok(())
}

pub(crate) fn run_with_client(client: reqwest::Client) {
    let configuration_directory = match desktop_configuration_directory() {
        Ok(directory) => directory,
        Err(error) => {
            tracing::error!(%error, "the desktop configuration directory could not be resolved");
            process::exit(2);
        }
    };
    let origin = match load_desktop_origin(&configuration_directory) {
        Ok(origin) => origin,
        Err(error) => {
            tracing::error!(%error, "the desktop origin configuration could not be loaded");
            process::exit(2);
        }
    };
    let preferences_directory = configuration_directory.clone();

    tauri::Builder::default()
        .manage(DesktopState {
            origin: Mutex::new(origin),
            configuration_directory,
            client,
            session: Arc::new(Mutex::new(DesktopSession::new(SecretServiceStore))),
            transports: Arc::new(Mutex::new(HashMap::new())),
            transport_generation: Mutex::new(0),
        })
        .setup(move |app| {
            let window = app
                .get_webview_window(MAIN_WINDOW_LABEL)
                .ok_or("the main window is unavailable")?;
            let preferences = DesktopPreferences::load(&preferences_directory);

            window.set_decorations(preferences.window_decorations())?;
            if let Err(error) = window.set_zoom(preferences.zoom_factor()) {
                tracing::warn!(%error, "the persisted zoom factor could not be applied at startup");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_session_status,
            desktop_get_origin,
            desktop_set_origin,
            desktop_get_window_decorations,
            desktop_set_window_decorations,
            desktop_get_zoom,
            desktop_set_zoom,
            desktop_auth_login,
            desktop_auth_resume,
            desktop_auth_me,
            desktop_auth_logout,
            desktop_api_request,
            desktop_read_clipboard_image,
            desktop_workspace_events_subscribe,
            desktop_workspace_events_stop
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            tracing::error!(%error, "the desktop application host failed");
            process::exit(1);
        });
}

fn load_desktop_origin(directory: &std::path::Path) -> Result<String, DesktopError> {
    match env::var("ATLAS_DESKTOP_ORIGIN") {
        Ok(origin) => {
            let configuration = DesktopConfiguration::from_selected_origin(&origin)?;
            configuration.save(directory)?;
            Ok(configuration.origin().to_owned())
        }
        Err(_) => match DesktopConfiguration::load(directory) {
            Ok(configuration) => Ok(configuration.origin().to_owned()),
            Err(DesktopError::ConfigurationUnavailable) => {
                let configuration =
                    DesktopConfiguration::from_selected_origin(DEFAULT_DESKTOP_ORIGIN)?;
                configuration.save(directory)?;
                Ok(configuration.origin().to_owned())
            }
            Err(error) => Err(error),
        },
    }
}

fn desktop_configuration_directory() -> Result<PathBuf, DesktopError> {
    if let Some(directory) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(directory).join("atlas-desktop"));
    }

    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config").join("atlas-desktop"))
        .ok_or(DesktopError::ConfigurationUnavailable)
}

/// Default CSS reference resolution, matching the value WebKitGTK divides by to
/// derive its device scale factor.
#[cfg(target_os = "linux")]
const FALLBACK_SCREEN_DPI: f64 = 96.0;

/// Repairs the GDK screen resolution when no settings provider supplies one.
///
/// Under the Wayland backend GDK sources the resolution from an XSETTINGS
/// provider or a desktop portal, and reports `-1` when neither is present, as
/// on bare wlroots compositors. WebKitGTK does not guard against that sentinel
/// and derives `scale = resolution / 96`, so a negative scale factor reaches
/// layout and corrupts every length in the webview. Must run before the webview
/// is created, and only overrides values that are already invalid, so an
/// explicitly configured resolution is always left untouched.
#[cfg(target_os = "linux")]
fn ensure_valid_screen_resolution() {
    if gtk::init().is_err() {
        return;
    }

    let Some(screen) = gtk::gdk::Screen::default() else {
        return;
    };

    let resolution = screen.resolution();
    if !resolution.is_finite() || resolution <= 0.0 {
        screen.set_resolution(FALLBACK_SCREEN_DPI);
    }
}

#[allow(dead_code)]
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,atlas_desktop=debug".into()),
        )
        .with_target(true)
        .init();

    #[cfg(target_os = "linux")]
    ensure_valid_screen_resolution();

    let client = match ReqwestTransportFactory::system().client() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "the desktop HTTP client could not be constructed");
            process::exit(1);
        }
    };

    run_with_client(client);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod command_tests {
    use super::*;
    use tauri::{
        WebviewWindowBuilder,
        ipc::CallbackFn,
        test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets},
        webview::InvokeRequest,
    };

    fn test_state() -> DesktopState {
        test_state_with_directory(std::env::temp_dir())
    }

    fn test_state_with_directory(configuration_directory: PathBuf) -> DesktopState {
        DesktopState {
            origin: Mutex::new("https://atlas.example.test".to_owned()),
            configuration_directory,
            client: reqwest::Client::new(),
            session: Arc::new(Mutex::new(DesktopSession::new(SecretServiceStore))),
            transports: Arc::new(Mutex::new(HashMap::new())),
            transport_generation: Mutex::new(0),
        }
    }

    fn pending_transport(generation: u64) -> TransportEntry {
        TransportEntry {
            generation,
            handle: tauri::async_runtime::spawn(std::future::pending::<()>()),
        }
    }

    fn preferences_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atlas-desktop-preferences-command-{label}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn desktop_auth_login_deserializes_the_exact_vue_credentials_payload() {
        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_auth_login])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_auth_login".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                "credentials": {"username": "desktop-user", "password": ""}
            })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["error"], "desktop authentication failed");
    }

    #[test]
    fn desktop_auth_resume_executes_through_the_tauri_command_boundary() {
        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_auth_resume])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_auth_resume".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the resume command invokes through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["error"], "desktop session is unavailable");
    }

    #[test]
    fn desktop_get_window_decorations_resolves_the_safe_default_when_unset() {
        let directory = preferences_test_directory("get-default");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_get_window_decorations])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_get_window_decorations".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["data"]["window_decorations"], true);
    }

    #[test]
    fn desktop_set_window_decorations_persists_and_returns_the_updated_preference() {
        let directory = preferences_test_directory("set-persists");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_set_window_decorations])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_set_window_decorations".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "decorations": false })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["data"]["window_decorations"], false);
        let persisted = std::fs::read_to_string(directory.join("preferences.json"))
            .expect("preferences are persisted");
        assert_eq!(
            persisted,
            "{\"window_decorations\":false,\"zoom_factor\":1.0}\n"
        );

        std::fs::remove_dir_all(&directory).expect("temporary preferences are removed");
    }

    #[test]
    fn desktop_set_window_decorations_fails_closed_when_the_main_window_is_missing() {
        let directory = preferences_test_directory("set-missing-window");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_set_window_decorations])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, "not-main", Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_set_window_decorations".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "decorations": false })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["error"], "desktop window is unavailable");
        assert!(!directory.join("preferences.json").exists());
    }

    #[test]
    fn desktop_get_zoom_resolves_the_default_zoom_when_unset() {
        let directory = preferences_test_directory("get-zoom-default");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_get_zoom])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_get_zoom".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["data"]["zoom_factor"], 1.0);
    }

    #[test]
    fn desktop_set_zoom_persists_and_returns_the_updated_zoom() {
        let directory = preferences_test_directory("set-zoom-persists");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_set_zoom])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_set_zoom".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "zoomFactor": 1.5 })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["data"]["zoom_factor"], 1.5);
        let persisted = std::fs::read_to_string(directory.join("preferences.json"))
            .expect("preferences are persisted");
        assert_eq!(
            persisted,
            "{\"window_decorations\":true,\"zoom_factor\":1.5}\n"
        );

        std::fs::remove_dir_all(&directory).expect("temporary preferences are removed");
    }

    #[test]
    fn desktop_set_zoom_clamps_an_out_of_range_request() {
        let directory = preferences_test_directory("set-zoom-clamps");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_set_zoom])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_set_zoom".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "zoomFactor": 9.0 })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["data"]["zoom_factor"], 3.0);

        std::fs::remove_dir_all(&directory).expect("temporary preferences are removed");
    }

    #[test]
    fn desktop_set_zoom_fails_closed_when_the_main_window_is_missing() {
        let directory = preferences_test_directory("set-zoom-missing-window");
        let app = mock_builder()
            .manage(test_state_with_directory(directory.clone()))
            .invoke_handler(tauri::generate_handler![desktop_set_zoom])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, "not-main", Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_set_zoom".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "zoomFactor": 1.5 })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the command must deserialize and invoke through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["error"], "desktop window is unavailable");
        assert!(!directory.join("preferences.json").exists());
    }

    #[tokio::test]
    async fn active_identity_failure_cancels_only_the_matching_origin_and_emits_scoped_auth_loss() {
        let state = test_state();
        let matching_scope =
            SessionScope::new("https://atlas.example.test", "user-1").expect("matching scope");
        let other_scope =
            SessionScope::new("https://other.example.test", "user-2").expect("other scope");
        let matching_key = (scope_transport_key(&matching_scope), "ws-a".to_owned());
        let other_key = (scope_transport_key(&other_scope), "ws-b".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .extend([
                (matching_key.clone(), pending_transport(1)),
                (other_key.clone(), pending_transport(1)),
            ]);
        let mut actions = Vec::new();

        let result = scope_for_active_identity_or_fail_closed_with(
            &state,
            |_| Err("desktop session is unavailable"),
            |action| {
                actions.push(action);
                Ok(())
            },
        );

        assert_eq!(result, Err("desktop session is unavailable"));

        assert_eq!(
            actions,
            vec![IpcSessionAction {
                origin: "https://atlas.example.test".to_owned(),
                identity: None,
                cancel_transport: true,
            }]
        );
        let transports = state.transports.lock().expect("transport map available");
        assert!(!transports.contains_key(&matching_key));
        assert!(transports.contains_key(&other_key));
    }

    #[tokio::test]
    async fn a_late_stop_for_a_previous_workspace_preserves_the_replacing_subscription() {
        let state = test_state();
        let scope = SessionScope::new("https://atlas.example.test", "user-1").expect("scope");
        let previous_key = (scope_transport_key(&scope), "ws-previous".to_owned());
        let replacing_key = (scope_transport_key(&scope), "ws-replacing".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .extend([
                (previous_key.clone(), pending_transport(1)),
                (replacing_key.clone(), pending_transport(1)),
            ]);

        cancel_workspace_transport(&state, &scope, "ws-previous", None).expect("stop cancels");

        let transports = state.transports.lock().expect("transport map available");
        assert!(!transports.contains_key(&previous_key));
        assert!(transports.contains_key(&replacing_key));
    }

    #[tokio::test]
    async fn a_late_stop_for_an_old_generation_preserves_the_replacing_stream() {
        let state = test_state();
        let scope = SessionScope::new("https://atlas.example.test", "user-1").expect("scope");
        let key = (scope_transport_key(&scope), "ws-a".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .insert(key.clone(), pending_transport(2));

        cancel_workspace_transport(&state, &scope, "ws-a", Some(1)).expect("stale stop is ignored");

        let transports = state.transports.lock().expect("transport map available");
        let entry = transports
            .get(&key)
            .expect("generation-2 stream must remain registered");
        assert_eq!(entry.generation, 2);
    }

    #[tokio::test]
    async fn a_matching_generation_stop_removes_the_stream() {
        let state = test_state();
        let scope = SessionScope::new("https://atlas.example.test", "user-1").expect("scope");
        let key = (scope_transport_key(&scope), "ws-a".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .insert(key.clone(), pending_transport(3));

        cancel_workspace_transport(&state, &scope, "ws-a", Some(3)).expect("matching stop cancels");

        let transports = state.transports.lock().expect("transport map available");
        assert!(!transports.contains_key(&key));
    }

    #[tokio::test]
    async fn cancelling_a_scope_removes_every_workspace_stream_it_owns() {
        let state = test_state();
        let scope = SessionScope::new("https://atlas.example.test", "user-1").expect("scope");
        let other_scope =
            SessionScope::new("https://atlas.example.test", "user-2").expect("other scope");
        let first_key = (scope_transport_key(&scope), "ws-a".to_owned());
        let second_key = (scope_transport_key(&scope), "ws-b".to_owned());
        let other_key = (scope_transport_key(&other_scope), "ws-a".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .extend([
                (first_key.clone(), pending_transport(1)),
                (second_key.clone(), pending_transport(1)),
                (other_key.clone(), pending_transport(1)),
            ]);

        cancel_transport(&state, &scope).expect("scope cancel");

        let transports = state.transports.lock().expect("transport map available");
        assert!(!transports.contains_key(&first_key));
        assert!(!transports.contains_key(&second_key));
        assert!(transports.contains_key(&other_key));
    }

    #[test]
    fn desktop_auth_logout_fails_closed_through_the_command_boundary_without_a_session() {
        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_auth_logout])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_auth_logout".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let response = get_ipc_response(&webview, request)
            .expect("the logout command invokes through Tauri IPC")
            .deserialize::<serde_json::Value>()
            .expect("the command response is JSON");

        assert_eq!(response["error"], "desktop session is unavailable");
    }

    fn decode_framed_http_response(framed: &[u8]) -> (serde_json::Value, Vec<u8>) {
        let meta_length =
            u32::from_le_bytes(framed[0..4].try_into().expect("length prefix present")) as usize;
        let meta: serde_json::Value =
            serde_json::from_slice(&framed[4..4 + meta_length]).expect("meta decodes as JSON");
        let body = framed[4 + meta_length..].to_vec();

        (meta, body)
    }

    #[test]
    fn frame_ipc_http_response_round_trips_status_headers_and_body() {
        let meta = IpcHttpResponseMeta {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-request-id".to_owned(), "request-1".to_owned()),
            ],
        };
        let body = b"{\"ok\":true}".to_vec();

        let framed = frame_ipc_http_response(&meta, &body).expect("framing succeeds");
        let (decoded_meta, decoded_body) = decode_framed_http_response(&framed);

        assert_eq!(decoded_meta["status"], 200);
        assert_eq!(decoded_meta["headers"][0][0], "content-type");
        assert_eq!(decoded_meta["headers"][0][1], "application/json");
        assert_eq!(decoded_meta["headers"][1][0], "x-request-id");
        assert_eq!(decoded_meta["headers"][1][1], "request-1");
        assert_eq!(decoded_body, body);
    }

    #[test]
    fn frame_ipc_http_response_round_trips_an_empty_body() {
        let meta = IpcHttpResponseMeta {
            status: 204,
            headers: Vec::new(),
        };

        let framed = frame_ipc_http_response(&meta, &[]).expect("framing succeeds");
        let (decoded_meta, decoded_body) = decode_framed_http_response(&framed);

        assert_eq!(decoded_meta["status"], 204);
        assert_eq!(decoded_meta["headers"], serde_json::json!([]));
        assert!(decoded_body.is_empty());
    }

    #[test]
    fn desktop_api_request_rejects_through_the_command_boundary_without_a_session() {
        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_api_request])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, Default::default())
            .build()
            .expect("the command test webview builds");
        let request = InvokeRequest {
            cmd: "desktop_api_request".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("valid test URL"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                "request": {
                    "method": "GET",
                    "path": "/api/auth/me",
                    "headers": [],
                    "body": []
                }
            })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        let rejection = get_ipc_response(&webview, request)
            .expect_err("the API request command must fail closed without a session");

        assert_eq!(
            rejection,
            serde_json::Value::String("desktop session is unavailable".to_owned())
        );
    }

    #[tokio::test]
    async fn an_unauthorized_api_response_invalidates_the_scope_and_emits_the_session_action() {
        use tauri::Listener;

        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_api_request])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let state = app.state::<DesktopState>();
        let scope = SessionScope::new("https://atlas.example.test", "user-1").expect("scope");
        let key = (scope_transport_key(&scope), "ws-a".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .insert(key.clone(), pending_transport(1));
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        app.listen("atlas://session-action", move |event| {
            sink.lock()
                .expect("captured actions available")
                .push(event.payload().to_owned());
        });

        invalidate_scope(&state, app.handle(), &scope)
            .expect("the 401 branch invalidates the scope");

        assert!(
            !state
                .transports
                .lock()
                .expect("transport map available")
                .contains_key(&key)
        );
        let payloads = received.lock().expect("captured actions available");
        assert_eq!(payloads.len(), 1);
        let action: serde_json::Value =
            serde_json::from_str(&payloads[0]).expect("the session action is JSON");
        assert_eq!(action["origin"], "https://atlas.example.test");
        assert_eq!(action["identity"], "user-1");
        assert_eq!(action["cancel_transport"], true);
    }

    #[test]
    fn resume_identity_rejects_unexpected_sensitive_response_fields() {
        let response = serde_json::json!({
            "principal_type": "user",
            "username": "desktop-user",
            "email": null,
            "id": "019ef171-bbcf-7b90-9be6-5dbb382afd08",
            "display_name": null,
            "is_root": false,
            "is_system_admin": false,
            "agent": null,
            "token": "must-never-cross-ipc"
        });

        assert!(parse_desktop_identity(response).is_err());
    }

    #[test]
    fn tauri_invoke_boundary_returns_typed_identity_and_rejects_sensitive_fields() {
        let app = mock_builder()
            .manage(test_state())
            .invoke_handler(tauri::generate_handler![desktop_test_identity])
            .build(mock_context(noop_assets()))
            .expect("the command test app builds");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("the command test webview builds");
        let identity = serde_json::json!({
            "principal_type": "user",
            "username": "desktop-user",
            "email": null,
            "id": "019ef171-bbcf-7b90-9be6-5dbb382afd08",
            "display_name": "Desktop User",
            "is_root": false,
            "is_system_admin": false,
            "agent": null
        });

        let success = get_ipc_response(
            &webview,
            InvokeRequest {
                cmd: "desktop_test_identity".into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().expect("valid test URL"),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "identity": identity })),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_owned(),
            },
        )
        .expect("the identity command invokes through Tauri IPC")
        .deserialize::<serde_json::Value>()
        .expect("the command response is JSON");

        assert_eq!(success["data"]["username"], "desktop-user");
        assert!(success["data"].get("token").is_none());

        let rejected = get_ipc_response(
            &webview,
            InvokeRequest {
                cmd: "desktop_test_identity".into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().expect("valid test URL"),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                    "identity": {
                        "principal_type": "user",
                        "username": "desktop-user",
                        "email": null,
                        "id": null,
                        "display_name": null,
                        "is_root": false,
                        "is_system_admin": false,
                        "agent": null,
                        "token": "must-never-cross-ipc"
                    }
                })),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_owned(),
            },
        )
        .expect("the identity command invokes through Tauri IPC")
        .deserialize::<serde_json::Value>()
        .expect("the command response is JSON");

        assert_eq!(rejected["data"], serde_json::Value::Null);
        assert_eq!(rejected["error"], "desktop session is invalid");
    }

    #[test]
    fn failed_resume_attempts_every_cleanup_step_even_when_each_prior_step_fails() {
        for failing_step in 0..4 {
            let attempts = std::cell::Cell::new([0_u8; 4]);
            let result = run_failed_resume_cleanup(
                || {
                    let mut values = attempts.get();
                    values[0] += 1;
                    attempts.set(values);
                    (failing_step != 0).then_some(()).ok_or("cancel failed")
                },
                || {
                    let mut values = attempts.get();
                    values[1] += 1;
                    attempts.set(values);
                    (failing_step != 1).then_some(()).ok_or("delete failed")
                },
                || {
                    let mut values = attempts.get();
                    values[2] += 1;
                    attempts.set(values);
                    (failing_step != 2).then_some(()).ok_or("action failed")
                },
                || {
                    let mut values = attempts.get();
                    values[3] += 1;
                    attempts.set(values);
                    (failing_step != 3).then_some(()).ok_or("identity failed")
                },
            );

            assert_eq!(attempts.get(), [1, 1, 1, 1]);
            assert_eq!(result, Err("desktop session cleanup failed"));
        }
    }

    #[tokio::test]
    async fn failed_resume_cleanup_attempts_every_action_for_one_origin_and_preserves_another() {
        let state = test_state();
        let failed_scope =
            SessionScope::new("https://atlas.example.test", "user-1").expect("failed scope");
        let surviving_scope =
            SessionScope::new("https://other.example.test", "user-2").expect("surviving scope");
        let failed_key = (scope_transport_key(&failed_scope), "ws-a".to_owned());
        let surviving_key = (scope_transport_key(&surviving_scope), "ws-b".to_owned());
        state
            .transports
            .lock()
            .expect("transport map available")
            .extend([
                (failed_key.clone(), pending_transport(1)),
                (surviving_key.clone(), pending_transport(1)),
            ]);
        let attempts = std::cell::Cell::new([0_u8; 3]);

        let result = run_failed_resume_cleanup_for_scope(
            &state,
            &failed_scope,
            || {
                let mut values = attempts.get();
                values[0] += 1;
                attempts.set(values);
                Err("delete failed")
            },
            || {
                let mut values = attempts.get();
                values[1] += 1;
                attempts.set(values);
                Err("action failed")
            },
            || {
                let mut values = attempts.get();
                values[2] += 1;
                attempts.set(values);
                Err("identity failed")
            },
        );

        assert_eq!(result, Err("desktop session cleanup failed"));
        assert_eq!(attempts.get(), [1, 1, 1]);
        let transports = state.transports.lock().expect("transport map available");
        assert!(!transports.contains_key(&failed_key));
        assert!(transports.contains_key(&surviving_key));
    }
}
