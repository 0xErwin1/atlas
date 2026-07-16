use crate::TransportFactory;
use std::{collections::HashSet, io, path::Path};

#[cfg(feature = "desktop-gate")]
use {
    axum::{
        Router,
        extract::State,
        http::{Request, StatusCode, header::ACCEPT},
        middleware::{self, Next},
        response::{IntoResponse, Response},
    },
    axum_server::{Handle, tls_rustls::RustlsConfig},
    rand::RngCore,
    rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose},
    std::{
        fs,
        io::{BufRead, BufReader, Write},
        net::{SocketAddr, TcpListener},
        os::unix::fs::FileTypeExt,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
        time::Duration,
    },
    tempfile::TempDir,
    thiserror::Error,
    tokio::{task::JoinHandle, time::timeout},
    zeroize::Zeroizing,
};

#[cfg(not(debug_assertions))]
compile_error!("desktop-gate is a non-shipping target");

pub struct GateTransportFactory {
    certificate: reqwest::Certificate,
}

impl GateTransportFactory {
    pub fn from_ca_pem(ca_pem: &[u8]) -> Result<Self, reqwest::Error> {
        Ok(Self {
            certificate: reqwest::Certificate::from_pem(ca_pem)?,
        })
    }
}

impl TransportFactory for GateTransportFactory {
    fn client(&self) -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder()
            .tls_certs_merge([self.certificate.clone()])
            .build()
    }
}

#[cfg(feature = "desktop-gate")]
const FAULT_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(feature = "desktop-gate")]
type PemBundle = (Vec<u8>, Vec<u8>, Vec<u8>);

#[cfg(feature = "desktop-gate")]
#[derive(Clone, Default)]
struct FaultControls {
    rest: FaultControl,
    workspace_sse: FaultControl,
}

#[cfg(feature = "desktop-gate")]
#[derive(Clone)]
struct FaultControl {
    resume: Arc<Mutex<tokio_util::sync::CancellationToken>>,
}

#[cfg(feature = "desktop-gate")]
impl Default for FaultControl {
    fn default() -> Self {
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();

        Self {
            resume: Arc::new(Mutex::new(token)),
        }
    }
}

#[cfg(feature = "desktop-gate")]
impl FaultControl {
    fn pause(&self) {
        let mut token = self
            .resume
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *token = tokio_util::sync::CancellationToken::new();
    }

    fn resume(&self) {
        let token = self
            .resume
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        token.cancel();
    }

    async fn wait(&self) -> bool {
        let token = self
            .resume
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();

        timeout(FAULT_TIMEOUT, token.cancelled()).await.is_ok()
    }
}

#[cfg(feature = "desktop-gate")]
async fn apply_fault_control(
    State(faults): State<FaultControls>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let control = match request.uri().path() {
        "/api/auth/me" => Some(&faults.rest),
        path if path.starts_with("/api/workspaces/") && path.ends_with("/events") => {
            Some(&faults.workspace_sse)
        }
        _ => None,
    };

    match control {
        Some(control) if !control.wait().await => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        _ => next.run(request).await,
    }
}

#[cfg(feature = "desktop-gate")]
#[derive(Debug, Error)]
pub enum GateServerError {
    #[error("desktop gate server setup failed")]
    Setup,
    #[error("desktop gate transport failed")]
    Transport,
    #[error("desktop gate server shutdown failed")]
    Shutdown,
}

/// Real TLS Atlas process used exclusively by the non-shipping desktop gate.
#[cfg(feature = "desktop-gate")]
pub struct TlsGateServer {
    ca_certificate_pem: Vec<u8>,
    origin: String,
    identity: atlas_server::desktop_gate_support::EphemeralIdentity,
    state: atlas_server::desktop_gate_support::AppState,
    faults: FaultControls,
    handle: Handle,
    server_task: JoinHandle<()>,
    database: atlas_test_db::TestDb,
}

#[cfg(feature = "desktop-gate")]
impl TlsGateServer {
    pub async fn spawn() -> Result<Self, GateServerError> {
        // Rustls permits this process-wide provider installation only once per test process.
        match rustls::crypto::ring::default_provider().install_default() {
            Ok(()) | Err(_) => {}
        }
        let database = atlas_test_db::TestDb::create()
            .await
            .map_err(|_| GateServerError::Setup)?;
        let identity = atlas_server::desktop_gate_support::seed_ephemeral_identity(database.conn())
            .await
            .map_err(|_| GateServerError::Setup)?;
        let state = atlas_server::desktop_gate_support::app_state(database.conn().clone())
            .await
            .map_err(|_| GateServerError::Setup)?;
        let (ca_certificate_pem, certificate_pem, private_key_pem) = generate_tls_material()?;
        let config = RustlsConfig::from_pem(certificate_pem, private_key_pem)
            .await
            .map_err(|_| GateServerError::Setup)?;
        let faults = FaultControls::default();
        let app = Router::new()
            .fallback_service(atlas_server::desktop_gate_support::app(state.clone()))
            .layer(middleware::from_fn_with_state(
                faults.clone(),
                apply_fault_control,
            ));
        let handle = Handle::new();
        let server_handle = handle.clone();
        let server_task = tokio::spawn(async move {
            let _ = axum_server::bind_rustls(SocketAddr::from(([127, 0, 0, 1], 0)), config)
                .handle(server_handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await;
        });
        let address = timeout(Duration::from_secs(2), handle.listening())
            .await
            .ok()
            .flatten()
            .ok_or(GateServerError::Setup)?;

        Ok(Self {
            ca_certificate_pem,
            origin: format!("https://localhost:{}", address.port()),
            identity,
            state,
            faults,
            handle,
            server_task,
            database,
        })
    }

    pub fn ca_certificate_pem(&self) -> &[u8] {
        &self.ca_certificate_pem
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn identity_credentials(&self) -> (&str, &str) {
        (&self.identity.username, &self.identity.password)
    }

    pub async fn login(&self, client: reqwest::Client) -> Result<GateSession, GateServerError> {
        let response = client
            .post(format!("{}/api/auth/login", self.origin))
            .json(&serde_json::json!({
                "username": self.identity.username,
                "password": self.identity.password,
            }))
            .send()
            .await
            .map_err(|_| GateServerError::Transport)?;

        if !response.status().is_success() {
            return Err(GateServerError::Transport);
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|_| GateServerError::Transport)?;
        let bearer = payload
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or(GateServerError::Transport)?
            .to_owned();

        Ok(GateSession {
            client,
            origin: self.origin.clone(),
            workspace_slug: self.identity.workspace_slug.clone(),
            bearer,
        })
    }

    pub fn pause_rest(&self) {
        self.faults.rest.pause();
    }

    pub fn resume_rest(&self) {
        self.faults.rest.resume();
    }

    pub fn pause_workspace_sse(&self) {
        self.faults.workspace_sse.pause();
    }

    pub fn resume_workspace_sse(&self) {
        self.faults.workspace_sse.resume();
    }

    pub fn publish_workspace_event(&self) {
        use std::sync::Arc;

        self.state.live.publish(atlas_server::live::LiveEvent {
            workspace_id: self.identity.workspace_id,
            project_id: None,
            board_id: None,
            document_id: None,
            event_type: "presence.updated".to_owned(),
            payload: Arc::from("{\"event_type\":\"presence.updated\",\"data\":{}}"),
        });
    }

    pub async fn shutdown(self) -> Result<(), GateServerError> {
        self.handle.shutdown();
        timeout(Duration::from_secs(2), self.server_task)
            .await
            .map_err(|_| GateServerError::Shutdown)?
            .map_err(|_| GateServerError::Shutdown)?;
        self.database
            .teardown()
            .await
            .map_err(|_| GateServerError::Shutdown)
    }
}

/// Non-secret metadata a gate caller may use to configure the next process layer.
#[cfg(feature = "desktop-gate")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateControllerMetadata {
    origin: String,
}

#[cfg(feature = "desktop-gate")]
impl GateControllerMetadata {
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

/// In-memory state for the currently executing gate case.
#[cfg(feature = "desktop-gate")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateCaseState {
    Ready,
}

/// Process identifiers and paths retained only to verify controller teardown.
#[cfg(feature = "desktop-gate")]
#[derive(Debug, Clone)]
pub struct GateControllerTeardown {
    secret_service_root: PathBuf,
    keyring_pid: u32,
    session_bus_pid: u32,
    database_name: String,
    database_dropped: bool,
}

#[cfg(feature = "desktop-gate")]
impl GateControllerTeardown {
    pub fn secret_service_root(&self) -> &Path {
        &self.secret_service_root
    }

    pub fn keyring_pid(&self) -> u32 {
        self.keyring_pid
    }

    pub fn session_bus_pid(&self) -> u32 {
        self.session_bus_pid
    }

    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    pub fn database_dropped(&self) -> bool {
        self.database_dropped
    }
}

#[cfg(feature = "desktop-gate")]
#[derive(Debug, Error)]
pub enum GateControllerError {
    #[error("desktop gate controller setup failed")]
    Setup,
    #[error("desktop gate controller authentication failed")]
    Authentication,
    #[error("desktop gate controller teardown failed")]
    Teardown,
    #[error("desktop gate controller webdriver flow failed")]
    WebDriver,
    #[error("desktop gate controller webdriver flow failed at {0}")]
    WebDriverStage(&'static str),
    #[error("desktop gate controller webdriver transport or protocol failed")]
    WebDriverTransport,
    #[error("desktop gate controller probe readiness timed out")]
    ProbeReadinessTimeout,
    #[error("desktop gate controller probe snapshot could not be decoded")]
    ProbeSnapshotDecode,
    #[error("desktop gate controller probe snapshot did not reach the required state")]
    ProbeSnapshotMismatch,
    #[error("desktop gate controller evidence write failed")]
    Evidence,
}

/// Non-secret milestones observed through the production Vue/Tauri boundary.
#[cfg(feature = "desktop-gate")]
pub struct GateWebDriverResult {
    identity_observed: bool,
    protected_rest_verified: bool,
    workspace_event_observed: bool,
    first_workspace_event_count: u64,
    workspace_event_type: Option<String>,
    workspace_event_matches_subscription: bool,
    sse_reconnect_observed: bool,
    recovered_workspace_event_count: u64,
    auth_remains_valid_after_recovery: bool,
    resumed_without_credentials: bool,
    production_logout_verified: bool,
}

#[cfg(feature = "desktop-gate")]
impl GateWebDriverResult {
    pub fn identity_observed(&self) -> bool {
        self.identity_observed
    }
    pub fn protected_rest_verified(&self) -> bool {
        self.protected_rest_verified
    }
    pub fn workspace_event_observed(&self) -> bool {
        self.workspace_event_observed
    }
    pub fn first_workspace_event_count(&self) -> u64 {
        self.first_workspace_event_count
    }
    pub fn workspace_event_type(&self) -> Option<&str> {
        self.workspace_event_type.as_deref()
    }
    pub fn workspace_event_matches_subscription(&self) -> bool {
        self.workspace_event_matches_subscription
    }
    pub fn sse_reconnect_observed(&self) -> bool {
        self.sse_reconnect_observed
    }
    pub fn recovered_workspace_event_count(&self) -> u64 {
        self.recovered_workspace_event_count
    }
    pub fn auth_remains_valid_after_recovery(&self) -> bool {
        self.auth_remains_valid_after_recovery
    }
    pub fn resumed_without_credentials(&self) -> bool {
        self.resumed_without_credentials
    }
    pub fn production_logout_verified(&self) -> bool {
        self.production_logout_verified
    }
}

/// The seven non-secret outcomes required for the deterministic Linux gate.
#[cfg(feature = "desktop-gate")]
pub struct GateFinalResult {
    all_cases_passed: bool,
}

#[cfg(feature = "desktop-gate")]
impl GateFinalResult {
    pub fn all_cases_passed(&self) -> bool {
        self.all_cases_passed
    }
}

#[cfg(feature = "desktop-gate")]
#[derive(Default)]
struct GateWebDriverProcesses {
    xvfb: Option<Child>,
    webdriver: Option<Child>,
    application: Option<Child>,
}

#[cfg(feature = "desktop-gate")]
impl GateWebDriverProcesses {
    fn stop(&mut self) -> Result<(), GateControllerError> {
        for child in [&mut self.application, &mut self.webdriver, &mut self.xvfb] {
            if let Some(child) = child.as_mut() {
                stop_gate_process(child)?;
            }
            *child = None;
        }

        Ok(())
    }
}

#[cfg(feature = "desktop-gate")]
fn stop_gate_process(child: &mut Child) -> Result<(), GateControllerError> {
    if child
        .try_wait()
        .map_err(|_| GateControllerError::Teardown)?
        .is_none()
    {
        stop_gate_descendants(child.id())?;
        child.kill().map_err(|_| GateControllerError::Teardown)?;
        child.wait().map_err(|_| GateControllerError::Teardown)?;
    }

    Ok(())
}

#[cfg(feature = "desktop-gate")]
fn stop_gate_descendants(parent_pid: u32) -> Result<(), GateControllerError> {
    let output = Command::new("pgrep")
        .args(["-P", &parent_pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| GateControllerError::Teardown)?;

    if !output.status.success() {
        return Ok(());
    }

    let descendants =
        std::str::from_utf8(&output.stdout).map_err(|_| GateControllerError::Teardown)?;
    for descendant in descendants.lines() {
        let pid = descendant
            .parse()
            .map_err(|_| GateControllerError::Teardown)?;
        stop_gate_descendants(pid)?;
        Command::new("kill")
            .arg(descendant)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| GateControllerError::Teardown)?
            .success()
            .then_some(())
            .ok_or(GateControllerError::Teardown)?;
    }

    Ok(())
}

/// Owns the complete in-memory local gate foundation without exposing credentials.
#[cfg(feature = "desktop-gate")]
pub struct GateController {
    server: Option<TlsGateServer>,
    secret_service: Option<GateSecretServiceController>,
    evidence: GateEvidenceController,
    case_state: GateCaseState,
    webdriver_processes: GateWebDriverProcesses,
}

#[cfg(feature = "desktop-gate")]
impl GateController {
    pub async fn start() -> Result<Self, GateControllerError> {
        let server = TlsGateServer::spawn()
            .await
            .map_err(|_| GateControllerError::Setup)?;
        let secret_service = match GateSecretServiceController::start() {
            Ok(secret_service) => secret_service,
            Err(_) => {
                server
                    .shutdown()
                    .await
                    .map_err(|_| GateControllerError::Teardown)?;
                return Err(GateControllerError::Setup);
            }
        };

        if secret_service
            .write_ca(server.ca_certificate_pem())
            .is_err()
        {
            drop(secret_service);
            server
                .shutdown()
                .await
                .map_err(|_| GateControllerError::Teardown)?;
            return Err(GateControllerError::Setup);
        }

        Ok(Self {
            server: Some(server),
            secret_service: Some(secret_service),
            evidence: GateEvidenceController::new(["case", "outcome"]),
            case_state: GateCaseState::Ready,
            webdriver_processes: GateWebDriverProcesses::default(),
        })
    }

    pub fn metadata(&self) -> GateControllerMetadata {
        GateControllerMetadata {
            origin: self
                .server
                .as_ref()
                .map_or_else(String::new, |server| server.origin().to_owned()),
        }
    }

    pub fn ca_certificate_pem(&self) -> Option<&[u8]> {
        self.server.as_ref().map(TlsGateServer::ca_certificate_pem)
    }

    pub fn evidence(&self) -> &GateEvidenceController {
        &self.evidence
    }

    pub fn case_state(&self) -> GateCaseState {
        self.case_state
    }

    pub async fn verify_private_login(&self) -> Result<(), GateControllerError> {
        let server = self
            .server
            .as_ref()
            .ok_or(GateControllerError::Authentication)?;
        let certificate = self
            .ca_certificate_pem()
            .ok_or(GateControllerError::Authentication)?;
        let client = GateTransportFactory::from_ca_pem(certificate)
            .and_then(|factory| factory.client())
            .map_err(|_| GateControllerError::Authentication)?;
        let session = server
            .login(client)
            .await
            .map_err(|_| GateControllerError::Authentication)?;

        session
            .me()
            .await
            .map_err(|_| GateControllerError::Authentication)
    }

    pub async fn run_webdriver_login_and_restart(
        &mut self,
        application: &str,
    ) -> Result<GateWebDriverResult, GateControllerError> {
        let result = self
            .run_webdriver_login_and_restart_inner(application)
            .await;
        if result.is_err() {
            self.webdriver_processes.stop()?;
        }
        result
    }

    pub async fn run_final_gate(
        &mut self,
        application: &str,
        evidence_path: impl AsRef<Path>,
    ) -> Result<GateFinalResult, GateControllerError> {
        let webdriver = self.run_webdriver_login_and_restart(application).await?;
        if !webdriver.identity_observed()
            || !webdriver.protected_rest_verified()
            || !webdriver.workspace_event_observed()
            || !webdriver.resumed_without_credentials()
            || !webdriver.production_logout_verified()
        {
            return Err(GateControllerError::WebDriver);
        }

        self.verify_expiry_or_revocation().await?;
        self.verify_logout_revocation().await?;
        self.verify_remote_origin_rules()?;

        let origin = self
            .server
            .as_ref()
            .ok_or(GateControllerError::Authentication)?
            .origin()
            .to_owned();
        let cases = [
            GateEvidenceCase::passed("login"),
            GateEvidenceCase::passed("protected_rest"),
            GateEvidenceCase::passed("rust_bearer_sse"),
            GateEvidenceCase::passed("restart_persistence"),
            GateEvidenceCase::passed("expiry_or_revocation"),
            GateEvidenceCase::passed("logout"),
            GateEvidenceCase::passed("remote_origin"),
        ];
        self.evidence
            .write_final(evidence_path, &origin, &cases)
            .map_err(|_| GateControllerError::Evidence)?;

        Ok(GateFinalResult {
            all_cases_passed: cases.iter().all(GateEvidenceCase::passed_outcome),
        })
    }

    async fn verify_expiry_or_revocation(&self) -> Result<(), GateControllerError> {
        let server = self
            .server
            .as_ref()
            .ok_or(GateControllerError::Authentication)?;
        let client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
            .and_then(|factory| factory.client())
            .map_err(|_| GateControllerError::Authentication)?;
        let session = server
            .login(client)
            .await
            .map_err(|_| GateControllerError::Authentication)?;

        atlas_server::desktop_gate_support::revoke_ephemeral_sessions(
            server.database.conn(),
            &server.identity,
        )
        .await
        .map_err(|_| GateControllerError::Authentication)?;

        (session
            .me_status()
            .await
            .map_err(|_| GateControllerError::Authentication)?
            == StatusCode::UNAUTHORIZED)
            .then_some(())
            .ok_or(GateControllerError::Authentication)
    }

    async fn verify_logout_revocation(&self) -> Result<(), GateControllerError> {
        let server = self
            .server
            .as_ref()
            .ok_or(GateControllerError::Authentication)?;
        let client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
            .and_then(|factory| factory.client())
            .map_err(|_| GateControllerError::Authentication)?;
        let session = server
            .login(client)
            .await
            .map_err(|_| GateControllerError::Authentication)?;

        session
            .logout()
            .await
            .map_err(|_| GateControllerError::Authentication)?;
        (session
            .me_status()
            .await
            .map_err(|_| GateControllerError::Authentication)?
            == StatusCode::UNAUTHORIZED)
            .then_some(())
            .ok_or(GateControllerError::Authentication)
    }

    fn verify_remote_origin_rules(&self) -> Result<(), GateControllerError> {
        let origin = self
            .server
            .as_ref()
            .ok_or(GateControllerError::Authentication)?
            .origin();
        crate::DesktopConfiguration::from_selected_origin(origin)
            .map_err(|_| GateControllerError::Authentication)?;

        for invalid in [
            "http://localhost:8443",
            "https://user:password@localhost:8443",
            "https://localhost:8443/path",
            "https://localhost:8443?query=value",
            "https://localhost:443",
        ] {
            if crate::DesktopConfiguration::from_selected_origin(invalid).is_ok() {
                return Err(GateControllerError::Authentication);
            }
        }

        let normalized =
            crate::DesktopConfiguration::from_selected_origin("https://localhost:8443/")
                .map_err(|_| GateControllerError::Authentication)?;
        if normalized.origin() != "https://localhost:8443" {
            return Err(GateControllerError::Authentication);
        }

        Ok(())
    }

    pub fn webdriver_processes_are_stopped(&mut self) -> bool {
        self.webdriver_processes.xvfb.is_none()
            && self.webdriver_processes.webdriver.is_none()
            && self.webdriver_processes.application.is_none()
    }

    async fn run_webdriver_login_and_restart_inner(
        &mut self,
        application: &str,
    ) -> Result<GateWebDriverResult, GateControllerError> {
        let server = self.server.as_ref().ok_or(GateControllerError::WebDriver)?;
        let secret_service = self
            .secret_service
            .as_ref()
            .ok_or(GateControllerError::WebDriver)?;
        let (display, xvfb) =
            start_xvfb().map_err(|_| GateControllerError::WebDriverStage("xvfb"))?;
        let driver_port = unused_port()?;
        let native_port = unused_port()?;
        let native_driver = native_driver_path()?;
        let mut driver = Command::new("tauri-driver");
        driver
            .args([
                "--port",
                &driver_port.to_string(),
                "--native-port",
                &native_port.to_string(),
            ])
            .arg("--native-driver")
            .arg(native_driver)
            .env("DISPLAY", format!(":{display}"))
            .env("ATLAS_DESKTOP_ORIGIN", server.origin())
            .env("ATLAS_DESKTOP_GATE_CA_PATH", secret_service.ca_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        secret_service.configure_process(&mut driver);
        let driver = driver
            .spawn()
            .map_err(|_| GateControllerError::WebDriverStage("driver"))?;
        self.webdriver_processes.xvfb = Some(xvfb);
        self.webdriver_processes.webdriver = Some(driver);

        let endpoint = format!("http://127.0.0.1:{driver_port}");
        wait_for_webdriver(&endpoint)
            .await
            .map_err(|_| GateControllerError::WebDriverStage("driver-readiness"))?;
        let session = create_webdriver_session(&endpoint, application)
            .await
            .map_err(|_| GateControllerError::WebDriverStage("session"))?;
        wait_for_gate_live_update_probe(&endpoint, &session)
            .await
            .map_err(|error| match error {
                GateControllerError::ProbeReadinessTimeout => error,
                _ => GateControllerError::WebDriverTransport,
            })?;
        let credentials = server.identity_credentials();
        submit_vue_login(&endpoint, &session, credentials.0, credentials.1)
            .await
            .map_err(|_| GateControllerError::WebDriverStage("vue-login"))?;
        let identity = invoke_tauri(
            &endpoint,
            &session,
            "desktop_auth_me",
            serde_json::json!({}),
        )
        .await
        .map_err(|_| GateControllerError::WebDriverStage("identity"))?;
        if identity
            .pointer("/data/username")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return Err(GateControllerError::WebDriver);
        }
        let status = invoke_tauri(
            &endpoint,
            &session,
            "desktop_session_status",
            serde_json::json!({}),
        )
        .await?;
        if status
            .pointer("/authenticated")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            return Err(GateControllerError::WebDriver);
        }
        invoke_tauri(
            &endpoint,
            &session,
            "desktop_workspace_events_subscribe",
            serde_json::json!({"workspaceSlug": server.identity.workspace_slug}),
        )
        .await?;
        let before_first_event = gate_live_update_snapshot(&endpoint, &session)
            .await
            .map_err(|_| GateControllerError::ProbeSnapshotDecode)?;
        server.publish_workspace_event();
        let first_event = wait_for_gate_live_update_snapshot(&endpoint, &session, |snapshot| {
            snapshot.count == before_first_event.count + 1
                && snapshot.event_type.as_deref() == Some("presence.updated")
                && snapshot.workspace_slug.as_deref()
                    == Some(server.identity.workspace_slug.as_str())
        })
        .await
        .map_err(|_| GateControllerError::ProbeSnapshotMismatch)?;

        server.pause_workspace_sse();
        server.pause_rest();
        invoke_tauri(
            &endpoint,
            &session,
            "desktop_workspace_events_subscribe",
            serde_json::json!({"workspaceSlug": server.identity.workspace_slug}),
        )
        .await
        .map_err(|_| GateControllerError::WebDriverStage("live-update-reconnect-start"))?;
        wait_for_gate_live_update_snapshot(&endpoint, &session, |snapshot| {
            snapshot.status.as_deref() == Some("reconnecting")
        })
        .await
        .map_err(|_| GateControllerError::ProbeSnapshotMismatch)?;

        server.resume_workspace_sse();
        server.resume_rest();
        wait_for_gate_live_update_snapshot(&endpoint, &session, |snapshot| {
            matches!(snapshot.status.as_deref(), Some("reconnected" | "resync"))
        })
        .await
        .map_err(|_| GateControllerError::ProbeSnapshotMismatch)?;

        let before_recovered_event = gate_live_update_snapshot(&endpoint, &session)
            .await
            .map_err(|_| GateControllerError::ProbeSnapshotDecode)?;
        server.publish_workspace_event();
        let recovered_event = wait_for_gate_live_update_snapshot(&endpoint, &session, |snapshot| {
            snapshot.count == before_recovered_event.count + 1
                && snapshot.event_type.as_deref() == Some("presence.updated")
                && snapshot.workspace_slug.as_deref()
                    == Some(server.identity.workspace_slug.as_str())
        })
        .await
        .map_err(|_| GateControllerError::ProbeSnapshotMismatch)?;
        let identity_after_recovery = invoke_tauri(
            &endpoint,
            &session,
            "desktop_auth_me",
            serde_json::json!({}),
        )
        .await?;
        let auth_remains_valid_after_recovery = identity_after_recovery
            .pointer("/data/username")
            .and_then(serde_json::Value::as_str)
            .is_some();
        if !auth_remains_valid_after_recovery {
            return Err(GateControllerError::WebDriver);
        }
        delete_webdriver_session(&endpoint, &session).await?;
        let resumed_session = create_webdriver_session(&endpoint, application).await?;
        let resumed = invoke_tauri(
            &endpoint,
            &resumed_session,
            "desktop_auth_resume",
            serde_json::json!({}),
        )
        .await?;
        if resumed
            .pointer("/data/username")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return Err(GateControllerError::WebDriver);
        }
        let _logout = invoke_tauri(
            &endpoint,
            &resumed_session,
            "desktop_auth_logout",
            serde_json::json!({}),
        )
        .await?;
        let resumed_after_logout = invoke_tauri(
            &endpoint,
            &resumed_session,
            "desktop_auth_resume",
            serde_json::json!({}),
        )
        .await?;
        if resumed_after_logout
            .pointer("/data/username")
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            return Err(GateControllerError::WebDriver);
        }
        delete_webdriver_session(&endpoint, &resumed_session).await?;
        Ok(GateWebDriverResult {
            identity_observed: true,
            protected_rest_verified: true,
            workspace_event_observed: first_event.count == before_first_event.count + 1,
            first_workspace_event_count: first_event.count,
            workspace_event_type: first_event.event_type,
            workspace_event_matches_subscription: first_event.workspace_slug.as_deref()
                == Some(server.identity.workspace_slug.as_str()),
            sse_reconnect_observed: true,
            recovered_workspace_event_count: recovered_event.count,
            auth_remains_valid_after_recovery,
            resumed_without_credentials: true,
            production_logout_verified: true,
        })
    }

    pub async fn shutdown(mut self) -> Result<GateControllerTeardown, GateControllerError> {
        let secret_service = self
            .secret_service
            .take()
            .ok_or(GateControllerError::Teardown)?;
        let mut teardown = GateControllerTeardown {
            secret_service_root: secret_service.root.path().to_path_buf(),
            keyring_pid: secret_service.login.id(),
            session_bus_pid: secret_service.session_bus_pid,
            database_name: self
                .server
                .as_ref()
                .map_or_else(String::new, |server| server.database.name().to_owned()),
            database_dropped: false,
        };

        let secret_service_result = stop_secret_service(secret_service);
        let webdriver_result = self.webdriver_processes.stop();
        let server_result = match self.server.take() {
            Some(server) => server
                .shutdown()
                .await
                .map_err(|_| GateControllerError::Teardown),
            None => Err(GateControllerError::Teardown),
        };

        secret_service_result?;
        webdriver_result?;
        server_result?;

        teardown.database_dropped = true;

        Ok(teardown)
    }
}

#[cfg(feature = "desktop-gate")]
fn unused_port() -> Result<u16, GateControllerError> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|_| GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
fn start_xvfb() -> Result<(String, Child), GateControllerError> {
    let mut child = Command::new("Xvfb")
        .args([
            "-displayfd",
            "1",
            "-screen",
            "0",
            "1200x800x24",
            "-nolisten",
            "tcp",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GateControllerError::WebDriver)?;
    let stdout = child.stdout.take().ok_or(GateControllerError::WebDriver)?;
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .map_err(|_| GateControllerError::WebDriver)?;
    let display = line.trim().to_owned();
    if display.is_empty() {
        return Err(GateControllerError::WebDriver);
    }
    Ok((display, child))
}

#[cfg(feature = "desktop-gate")]
fn native_driver_path() -> Result<PathBuf, GateControllerError> {
    let output = Command::new("which")
        .arg("WebKitWebDriver")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| GateControllerError::WebDriver)?;
    if !output.status.success() {
        return Err(GateControllerError::WebDriver);
    }
    let path = std::str::from_utf8(&output.stdout).map_err(|_| GateControllerError::WebDriver)?;
    let path = PathBuf::from(path.trim());
    path.is_file()
        .then_some(path)
        .ok_or(GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
async fn wait_for_webdriver(endpoint: &str) -> Result<(), GateControllerError> {
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("{endpoint}/status"))
            .send()
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
async fn create_webdriver_session(
    endpoint: &str,
    application: &str,
) -> Result<String, GateControllerError> {
    let response = reqwest::Client::new()
        .post(format!("{endpoint}/session"))
        .json(&serde_json::json!({
            "capabilities": {"alwaysMatch": {"browserName": "wry", "tauri:options": {"application": application}}}
        }))
        .send()
        .await
        .map_err(|_| GateControllerError::WebDriver)?;
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| GateControllerError::WebDriver)?;
    value
        .pointer("/value/sessionId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
async fn delete_webdriver_session(
    endpoint: &str,
    session: &str,
) -> Result<(), GateControllerError> {
    reqwest::Client::new()
        .delete(format!("{endpoint}/session/{session}"))
        .send()
        .await
        .map_err(|_| GateControllerError::WebDriver)?
        .status()
        .is_success()
        .then_some(())
        .ok_or(GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
async fn submit_vue_login(
    endpoint: &str,
    session: &str,
    username: &str,
    password: &str,
) -> Result<(), GateControllerError> {
    let script = "const username = arguments[0]; const password = arguments[1]; const done = arguments[2]; const attempt = () => { const user = document.getElementById('username'); const pass = document.getElementById('password'); const form = user?.closest('form'); if (!user || !pass || !form) { setTimeout(attempt, 50); return; } user.value = username; user.dispatchEvent(new Event('input', { bubbles: true })); pass.value = password; pass.dispatchEvent(new Event('input', { bubbles: true })); form.requestSubmit(); const check = () => window.__TAURI_INTERNALS__.invoke('desktop_session_status').then((status) => { if (status.authenticated) done(true); else setTimeout(check, 50); }).catch(() => setTimeout(check, 50)); check(); }; attempt();";
    let value = execute_webdriver_async(
        endpoint,
        session,
        script,
        serde_json::json!([username, password]),
    )
    .await?;
    (value == serde_json::Value::Bool(true))
        .then_some(())
        .ok_or(GateControllerError::WebDriver)
}

#[cfg(feature = "desktop-gate")]
async fn invoke_tauri(
    endpoint: &str,
    session: &str,
    command: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, GateControllerError> {
    let script = "const command = arguments[0]; const payload = arguments[1]; const done = arguments[2]; window.__TAURI_INTERNALS__.invoke(command, payload).then(done).catch(() => done(null));";
    execute_webdriver_async(
        endpoint,
        session,
        script,
        serde_json::json!([command, arguments]),
    )
    .await
}

#[cfg(feature = "desktop-gate")]
struct GateLiveUpdateSnapshot {
    count: u64,
    event_type: Option<String>,
    status: Option<String>,
    workspace_slug: Option<String>,
}

#[cfg(feature = "desktop-gate")]
async fn gate_live_update_snapshot(
    endpoint: &str,
    session: &str,
) -> Result<GateLiveUpdateSnapshot, GateControllerError> {
    let value = execute_webdriver_async(
        endpoint,
        session,
        "const done = arguments[0]; const observation = window.__atlasDesktopGateLiveUpdateObservation; Promise.resolve(observation?.snapshot()).then((snapshot) => done(snapshot ? { count: snapshot.count, eventType: snapshot.eventType, status: snapshot.status, workspaceSlug: snapshot.workspaceSlug } : null)).catch(() => done(null));",
        serde_json::json!([]),
    )
    .await?;

    let fields = value
        .as_object()
        .ok_or(GateControllerError::ProbeSnapshotDecode)?;
    let count = fields
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .ok_or(GateControllerError::ProbeSnapshotDecode)?;

    Ok(GateLiveUpdateSnapshot {
        count,
        event_type: fields
            .get("eventType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        status: fields
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        workspace_slug: fields
            .get("workspaceSlug")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(feature = "desktop-gate")]
async fn wait_for_gate_live_update_probe(
    endpoint: &str,
    session: &str,
) -> Result<(), GateControllerError> {
    const FRONTEND_READY_TIMEOUT: Duration = Duration::from_secs(15);
    let wait = async {
        loop {
            let value = execute_webdriver_async(
            endpoint,
            session,
            "const done = arguments[0]; done(Boolean(window.__atlasDesktopGateLiveUpdateObservation));",
            serde_json::json!([]),
        )
        .await?;
            if value == serde_json::Value::Bool(true) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };

    timeout(FRONTEND_READY_TIMEOUT, wait)
        .await
        .map_err(|_| GateControllerError::ProbeReadinessTimeout)?
}

#[cfg(feature = "desktop-gate")]
async fn wait_for_gate_live_update_snapshot<F>(
    endpoint: &str,
    session: &str,
    matches: F,
) -> Result<GateLiveUpdateSnapshot, GateControllerError>
where
    F: Fn(&GateLiveUpdateSnapshot) -> bool,
{
    for _ in 0..100 {
        let snapshot = gate_live_update_snapshot(endpoint, session).await?;
        if matches(&snapshot) {
            return Ok(snapshot);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Err(GateControllerError::ProbeSnapshotMismatch)
}

#[cfg(feature = "desktop-gate")]
async fn execute_webdriver_async(
    endpoint: &str,
    session: &str,
    script: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, GateControllerError> {
    let response = timeout(
        Duration::from_secs(15),
        reqwest::Client::new()
            .post(format!("{endpoint}/session/{session}/execute/async"))
            .json(&serde_json::json!({"script": script, "args": arguments}))
            .send(),
    )
    .await
    .map_err(|_| GateControllerError::WebDriverTransport)?
    .map_err(|_| GateControllerError::WebDriverTransport)?;
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| GateControllerError::WebDriverTransport)?;
    value
        .get("value")
        .cloned()
        .ok_or(GateControllerError::WebDriverTransport)
}

#[cfg(feature = "desktop-gate")]
fn stop_secret_service(
    mut secret_service: GateSecretServiceController,
) -> Result<(), GateControllerError> {
    let result = secret_service
        .stop()
        .map_err(|_| GateControllerError::Teardown);
    drop(secret_service);
    result
}

#[cfg(feature = "desktop-gate")]
pub struct GateSession {
    client: reqwest::Client,
    origin: String,
    workspace_slug: String,
    bearer: String,
}

#[cfg(feature = "desktop-gate")]
impl GateSession {
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    pub fn into_bearer(self) -> String {
        self.bearer
    }

    pub async fn me(&self) -> Result<(), GateServerError> {
        self.me_status()
            .await?
            .is_success()
            .then_some(())
            .ok_or(GateServerError::Transport)
    }

    pub async fn me_status(&self) -> Result<reqwest::StatusCode, GateServerError> {
        let response = self
            .client
            .get(format!("{}/api/auth/me", self.origin))
            .bearer_auth(&self.bearer)
            .send()
            .await
            .map_err(|_| GateServerError::Transport)?;

        Ok(response.status())
    }

    pub async fn logout(&self) -> Result<(), GateServerError> {
        crate::execute_bearer_logout(&self.client, &self.origin, &self.bearer)
            .await
            .map_err(|_| GateServerError::Transport)
    }

    pub async fn workspace_sse(&self) -> Result<(), GateServerError> {
        let response = timeout(
            FAULT_TIMEOUT + Duration::from_secs(1),
            self.client
                .get(format!(
                    "{}/api/workspaces/{}/events",
                    self.origin, self.workspace_slug
                ))
                .bearer_auth(&self.bearer)
                .header(ACCEPT, "text/event-stream")
                .send(),
        )
        .await
        .map_err(|_| GateServerError::Transport)?
        .map_err(|_| GateServerError::Transport)?;

        response
            .status()
            .is_success()
            .then_some(())
            .ok_or(GateServerError::Transport)
    }
}

#[cfg(feature = "desktop-gate")]
fn generate_tls_material() -> Result<PemBundle, GateServerError> {
    let mut ca_params = CertificateParams::new(Vec::new()).map_err(|_| GateServerError::Setup)?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let issuer = CertifiedIssuer::self_signed(
        ca_params,
        KeyPair::generate().map_err(|_| GateServerError::Setup)?,
    )
    .map_err(|_| GateServerError::Setup)?;
    let leaf_params =
        CertificateParams::new(vec!["localhost".to_owned()]).map_err(|_| GateServerError::Setup)?;
    let leaf_key = KeyPair::generate().map_err(|_| GateServerError::Setup)?;
    let leaf = leaf_params
        .signed_by(&leaf_key, &issuer)
        .map_err(|_| GateServerError::Setup)?;

    Ok((
        issuer.pem().into_bytes(),
        leaf.pem().into_bytes(),
        leaf_key.serialize_pem().into_bytes(),
    ))
}

/// Restricts gate evidence to caller-approved, non-sensitive diagnostic fields.
#[cfg(feature = "desktop-gate")]
#[derive(serde::Serialize)]
pub struct GateEvidenceCase {
    name: &'static str,
    outcome: &'static str,
}

#[cfg(feature = "desktop-gate")]
impl GateEvidenceCase {
    fn passed(name: &'static str) -> Self {
        Self {
            name,
            outcome: "pass",
        }
    }

    fn passed_outcome(&self) -> bool {
        self.outcome == "pass"
    }
}

pub struct GateEvidenceController {
    allowed_fields: HashSet<String>,
}

impl GateEvidenceController {
    pub fn new<const N: usize>(allowed_fields: [&str; N]) -> Self {
        Self {
            allowed_fields: allowed_fields.into_iter().map(str::to_owned).collect(),
        }
    }

    pub fn record<K, V, I>(&self, fields: I) -> String
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        fields
            .into_iter()
            .filter_map(|(key, value)| {
                let key = key.as_ref();
                self.allowed_fields.contains(key).then(|| {
                    let value: String = value
                        .as_ref()
                        .chars()
                        .filter(|character| !character.is_control())
                        .collect();
                    format!("{key}={value}")
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn write<K, V, I>(&self, path: impl AsRef<Path>, fields: I) -> io::Result<()>
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        std::fs::write(path, format!("{}\n", self.record(fields)))
    }

    #[cfg(feature = "desktop-gate")]
    pub fn write_final(
        &self,
        path: impl AsRef<Path>,
        origin: &str,
        cases: &[GateEvidenceCase],
    ) -> io::Result<()> {
        #[derive(serde::Serialize)]
        struct Evidence<'a> {
            schema: &'static str,
            build: &'static str,
            timestamp_unix_seconds: u64,
            origin_classification: &'static str,
            origin: &'a str,
            cases: &'a [GateEvidenceCase],
        }

        let timestamp_unix_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        let evidence = Evidence {
            schema: "atlas.desktop.linux-gate-evidence/v1",
            build: env!("CARGO_PKG_VERSION"),
            timestamp_unix_seconds,
            origin_classification: "canonical_https_nondefault_port",
            origin,
            cases,
        };
        let serialized = serde_json::to_vec(&evidence).map_err(io::Error::other)?;
        let path = path.as_ref();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serialized)?;
        file.write_all(b"\n")
    }
}

#[cfg(feature = "desktop-gate")]
const SECRET_SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(feature = "desktop-gate")]
const SECRET_SERVICE_READY_INTERVAL: Duration = Duration::from_millis(100);

/// Controls a private GNOME Keyring login collection without exposing its password.
#[cfg(feature = "desktop-gate")]
pub struct GateSecretServiceController {
    root: TempDir,
    paths: GateSecretServicePaths,
    password: Zeroizing<Vec<u8>>,
    login: Child,
    session_bus_pid: u32,
}

#[cfg(feature = "desktop-gate")]
struct GateSecretServicePaths {
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    runtime: PathBuf,
    control: PathBuf,
    session_bus_address: String,
}

#[cfg(feature = "desktop-gate")]
#[derive(Debug, Error)]
pub enum GateSecretServiceError {
    #[error("secret service gate setup failed")]
    Setup,
    #[error("secret service gate process failed")]
    Process,
    #[error("secret service login daemon exited before readiness")]
    LoginExited,
    #[error("secret service activation failed")]
    StartFailed,
    #[error("secret service gate did not become ready")]
    NotReady,
    #[error("secret service default alias was not the login collection")]
    LoginAliasUnavailable,
    #[error("secret service login collection remained locked")]
    LoginCollectionLocked,
    #[error("secret service gate session check failed")]
    Session,
}

#[cfg(feature = "desktop-gate")]
impl GateSecretServiceController {
    pub fn start() -> Result<Self, GateSecretServiceError> {
        let root = tempfile::Builder::new()
            .prefix("atlas-desktop-secret-service-")
            .tempdir()
            .map_err(|_| GateSecretServiceError::Setup)?;
        restrict_directory(root.path())?;

        let home = root.path().join("home");
        let data = root.path().join("data");
        let config = root.path().join("config");
        let cache = root.path().join("cache");
        let runtime = root.path().join("runtime");
        let control = runtime.join("keyring");
        for directory in [&home, &data, &config, &cache, &runtime, &control] {
            fs::create_dir(directory).map_err(|_| GateSecretServiceError::Setup)?;
            restrict_directory(directory)?;
        }
        let (session_bus_address, session_bus_pid) = start_session_bus(&runtime)?;
        let paths = GateSecretServicePaths {
            home,
            data,
            config,
            cache,
            runtime,
            control,
            session_bus_address,
        };

        let mut password = Zeroizing::new(vec![0_u8; 32]);
        rand::thread_rng().fill_bytes(&mut password);
        for byte in &mut *password {
            *byte = match *byte % 62 {
                value @ 0..=25 => b'a' + value,
                value @ 26..=51 => b'A' + (value - 26),
                value => b'0' + (value - 52),
            };
        }
        prepare_keyring_control(&paths.control)?;
        let mut login = match launch_keyring_login(&paths, &password) {
            Ok(login) => login,
            Err(error) => {
                stop_session_bus(session_bus_pid)?;
                return Err(error);
            }
        };
        if let Err(error) = start_keyring_service(&mut login, &paths)
            .and_then(|()| wait_for_login_collection(&mut login, &paths))
        {
            let login_result = stop_keyring_process(&mut login);
            let bus_result = stop_session_bus(session_bus_pid);

            return match (login_result, bus_result) {
                (Ok(()), Ok(())) => Err(error),
                _ => Err(GateSecretServiceError::Process),
            };
        }

        Ok(Self {
            root,
            paths,
            password,
            login,
            session_bus_pid,
        })
    }

    pub fn restart(&mut self) -> Result<(), GateSecretServiceError> {
        stop_keyring_process(&mut self.login)?;
        prepare_keyring_control(&self.paths.control)?;

        self.login = match launch_keyring_login(&self.paths, &self.password) {
            Ok(login) => login,
            Err(error) => {
                stop_session_bus(self.session_bus_pid)?;
                return Err(error);
            }
        };
        if let Err(error) = start_keyring_service(&mut self.login, &self.paths)
            .and_then(|()| wait_for_login_collection(&mut self.login, &self.paths))
        {
            let login_result = stop_keyring_process(&mut self.login);
            let bus_result = stop_session_bus(self.session_bus_pid);

            return match (login_result, bus_result) {
                (Ok(()), Ok(())) => Err(error),
                _ => Err(GateSecretServiceError::Process),
            };
        }

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), GateSecretServiceError> {
        stop_keyring_process(&mut self.login)
    }

    pub fn lock_default_collection(&self) -> Result<(), GateSecretServiceError> {
        let status = self
            .paths
            .configure(Command::new("gdbus"))
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.secrets",
                "--object-path",
                "/org/freedesktop/secrets",
                "--method",
                "org.freedesktop.Secret.Service.Lock",
                "['/org/freedesktop/secrets/collection/login']",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| GateSecretServiceError::Process)?;

        status
            .success()
            .then_some(())
            .ok_or(GateSecretServiceError::Process)
    }

    pub fn ca_path(&self) -> PathBuf {
        self.root.path().join("gate-ca.pem")
    }

    pub fn write_ca(&self, certificate: &[u8]) -> Result<(), GateSecretServiceError> {
        fs::write(self.ca_path(), certificate).map_err(|_| GateSecretServiceError::Setup)?;
        fs::set_permissions(self.ca_path(), fs::Permissions::from_mode(0o600))
            .map_err(|_| GateSecretServiceError::Setup)
    }

    pub fn configure_process(&self, command: &mut Command) {
        self.paths.apply(command);
    }
}

#[cfg(feature = "desktop-gate")]
impl GateSecretServicePaths {
    fn apply(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.session_bus_address)
            .env("GNOME_KEYRING_CONTROL", &self.control);
    }

    fn configure(&self, mut command: Command) -> Command {
        self.apply(&mut command);
        command
    }
}

#[cfg(feature = "desktop-gate")]
fn start_session_bus(runtime: &Path) -> Result<(String, u32), GateSecretServiceError> {
    let configuration = runtime.join("session.conf");
    fs::write(
        &configuration,
        format!(
            "<busconfig><type>session</type><listen>unix:dir={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow send_destination=\"*\"/><allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>",
            runtime.display()
        ),
    )
    .map_err(|_| GateSecretServiceError::Setup)?;
    let output = Command::new("dbus-daemon")
        .env("XDG_RUNTIME_DIR", runtime)
        .arg(format!("--config-file={}", configuration.display()))
        .args(["--fork", "--print-address=1", "--print-pid=1"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| GateSecretServiceError::Process)?;
    if !output.status.success() {
        return Err(GateSecretServiceError::StartFailed);
    }

    let output =
        std::str::from_utf8(&output.stdout).map_err(|_| GateSecretServiceError::Process)?;
    let mut lines = output.lines();
    let address = lines
        .next()
        .ok_or(GateSecretServiceError::Process)?
        .to_owned();
    let pid = lines
        .next()
        .ok_or(GateSecretServiceError::Process)?
        .parse()
        .map_err(|_| GateSecretServiceError::Process)?;

    if lines.next().is_some() || address.is_empty() {
        return Err(GateSecretServiceError::Process);
    }

    Ok((address, pid))
}

#[cfg(feature = "desktop-gate")]
fn stop_session_bus(pid: u32) -> Result<(), GateSecretServiceError> {
    let status = Command::new("kill")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| GateSecretServiceError::Process)?;

    status
        .success()
        .then_some(())
        .ok_or(GateSecretServiceError::Process)
}

#[cfg(feature = "desktop-gate")]
impl Drop for GateSecretServiceController {
    fn drop(&mut self) {
        if stop_keyring_process(&mut self.login).is_err() {
            // Drop cannot report a process-termination error.
        }
        if stop_session_bus(self.session_bus_pid).is_err() {
            // Drop cannot report a process-termination error.
        }
    }
}

#[cfg(feature = "desktop-gate")]
fn restrict_directory(directory: &Path) -> Result<(), GateSecretServiceError> {
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| GateSecretServiceError::Setup)
}

#[cfg(feature = "desktop-gate")]
fn prepare_keyring_control(control: &Path) -> Result<(), GateSecretServiceError> {
    if control.exists() {
        fs::remove_dir_all(control).map_err(|_| GateSecretServiceError::Setup)?;
    }

    fs::create_dir(control).map_err(|_| GateSecretServiceError::Setup)?;
    restrict_directory(control)
}

#[cfg(feature = "desktop-gate")]
fn launch_keyring_login(
    paths: &GateSecretServicePaths,
    password: &[u8],
) -> Result<Child, GateSecretServiceError> {
    let mut login = paths
        .configure(Command::new("gnome-keyring-daemon"))
        .args(["--foreground", "--login", "--components=secrets"])
        .arg(format!("--control-directory={}", paths.control.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GateSecretServiceError::Process)?;
    let mut stdin = login.stdin.take().ok_or(GateSecretServiceError::Process)?;
    stdin
        .write_all(password)
        .and_then(|()| stdin.flush())
        .map_err(|_| GateSecretServiceError::Process)?;
    drop(stdin);

    Ok(login)
}

#[cfg(feature = "desktop-gate")]
fn start_keyring_service(
    login: &mut Child,
    paths: &GateSecretServicePaths,
) -> Result<(), GateSecretServiceError> {
    wait_for_keyring_control(login, &paths.control.join("control"))?;

    let mut start = paths
        .configure(Command::new("gnome-keyring-daemon"))
        .args(["--start", "--components=secrets"])
        .arg(format!("--control-directory={}", paths.control.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GateSecretServiceError::Process)?;

    wait_for_child(&mut start, SECRET_SERVICE_READY_TIMEOUT)?
        .success()
        .then_some(())
        .ok_or(GateSecretServiceError::StartFailed)
}

#[cfg(feature = "desktop-gate")]
fn wait_for_keyring_control(
    login: &mut Child,
    socket: &Path,
) -> Result<(), GateSecretServiceError> {
    let deadline = std::time::Instant::now() + SECRET_SERVICE_READY_TIMEOUT;

    while std::time::Instant::now() < deadline {
        if login
            .try_wait()
            .map_err(|_| GateSecretServiceError::Process)?
            .is_some()
        {
            return Err(GateSecretServiceError::LoginExited);
        }

        if socket
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            return Ok(());
        }

        std::thread::sleep(SECRET_SERVICE_READY_INTERVAL);
    }

    Err(GateSecretServiceError::NotReady)
}

#[cfg(feature = "desktop-gate")]
fn wait_for_login_collection(
    login: &mut Child,
    paths: &GateSecretServicePaths,
) -> Result<(), GateSecretServiceError> {
    let deadline = std::time::Instant::now() + SECRET_SERVICE_READY_TIMEOUT;
    let mut found_login_alias = false;

    while std::time::Instant::now() < deadline {
        if login
            .try_wait()
            .map_err(|_| GateSecretServiceError::Process)?
            .is_some()
        {
            return Err(GateSecretServiceError::LoginExited);
        }

        let probe = paths
            .configure(Command::new("gdbus"))
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.secrets",
                "--object-path",
                "/org/freedesktop/secrets",
                "--method",
                "org.freedesktop.Secret.Service.ReadAlias",
                "default",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let ready = probe.is_ok_and(|output| {
            output.status.success()
                && std::str::from_utf8(&output.stdout).is_ok_and(|alias| {
                    alias.trim() == "(objectpath '/org/freedesktop/secrets/collection/login',)"
                })
        });
        if ready {
            found_login_alias = true;
            if login_collection_is_unlocked(paths) {
                return Ok(());
            }
        }

        std::thread::sleep(SECRET_SERVICE_READY_INTERVAL);
    }

    if found_login_alias {
        Err(GateSecretServiceError::LoginCollectionLocked)
    } else {
        Err(GateSecretServiceError::LoginAliasUnavailable)
    }
}

#[cfg(feature = "desktop-gate")]
fn login_collection_is_unlocked(paths: &GateSecretServicePaths) -> bool {
    paths
        .configure(Command::new("gdbus"))
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.secrets",
            "--object-path",
            "/org/freedesktop/secrets/collection/login",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            "org.freedesktop.Secret.Collection",
            "Locked",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && std::str::from_utf8(&output.stdout)
                    .is_ok_and(|locked| locked.trim() == "(<false>,)")
        })
}

#[cfg(feature = "desktop-gate")]
fn stop_keyring_process(login: &mut Child) -> Result<(), GateSecretServiceError> {
    if login
        .try_wait()
        .map_err(|_| GateSecretServiceError::Process)?
        .is_none()
    {
        let status = Command::new("kill")
            .args(["-TERM", &login.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| GateSecretServiceError::Process)?;
        if !status.success() {
            return Err(GateSecretServiceError::Process);
        }
        let _ = wait_for_child(login, SECRET_SERVICE_READY_TIMEOUT)?;
    }

    Ok(())
}

#[cfg(feature = "desktop-gate")]
fn wait_for_child(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, GateSecretServiceError> {
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| GateSecretServiceError::Process)?
        {
            return Ok(status);
        }

        std::thread::sleep(SECRET_SERVICE_READY_INTERVAL);
    }

    child.kill().map_err(|_| GateSecretServiceError::Process)?;
    child.wait().map_err(|_| GateSecretServiceError::Process)?;
    Err(GateSecretServiceError::Process)
}

#[cfg(all(test, feature = "desktop-gate"))]
mod tests {
    use super::*;
    use std::{
        os::unix::{fs::FileTypeExt, net::UnixListener},
        process::{Command, Stdio},
        thread,
    };

    #[test]
    fn control_socket_handshake_waits_for_a_live_login_process()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("control");
        let mut login = Command::new("sh")
            .args(["-c", "sleep 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let socket_for_thread = socket.clone();

        let listener = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            UnixListener::bind(socket_for_thread)
        });

        wait_for_keyring_control(&mut login, &socket)?;
        let listener = listener
            .join()
            .map_err(|_| "socket listener thread panicked")??;

        assert!(socket.symlink_metadata()?.file_type().is_socket());
        drop(listener);
        stop_keyring_process(&mut login)?;
        Ok(())
    }

    #[test]
    fn control_socket_handshake_rejects_an_exited_login_process()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("control");
        let mut login = Command::new("sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        assert!(matches!(
            wait_for_keyring_control(&mut login, &socket),
            Err(GateSecretServiceError::LoginExited)
        ));

        Ok(())
    }
}
