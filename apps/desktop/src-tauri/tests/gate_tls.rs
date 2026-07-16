#![cfg(feature = "desktop-gate")]

use atlas_desktop::{
    ReqwestTransportFactory, TransportFactory,
    gate::{GateSecretServiceController, GateTransportFactory},
};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};

#[tokio::test]
async fn generated_ca_is_required_for_real_tls_login_and_protected_rest()
-> Result<(), Box<dyn std::error::Error>> {
    let server = atlas_desktop::gate::TlsGateServer::spawn().await?;
    let production_client = ReqwestTransportFactory::system().client()?;

    assert!(server.login(production_client).await.is_err());

    let gate_client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
        .and_then(|factory| factory.client())?;
    let session = server.login(gate_client).await?;

    session.me().await?;
    drop(session);
    server.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn bounded_fault_controls_interrupt_and_recover_rest_and_workspace_sse()
-> Result<(), Box<dyn std::error::Error>> {
    let server = atlas_desktop::gate::TlsGateServer::spawn().await?;
    let gate_client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
        .and_then(|factory| factory.client())?;
    let session = server.login(gate_client).await?;

    server.pause_rest();
    assert!(session.me().await.is_err());
    server.resume_rest();
    session.me().await?;

    server.pause_workspace_sse();
    assert!(session.workspace_sse().await.is_err());
    server.resume_workspace_sse();
    session.workspace_sse().await?;
    drop(session);
    server.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn production_bearer_logout_revokes_the_real_atlas_session()
-> Result<(), Box<dyn std::error::Error>> {
    let server = atlas_desktop::gate::TlsGateServer::spawn().await?;
    let gate_client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
        .and_then(|factory| factory.client())?;
    let session = server.login(gate_client).await?;

    session.logout().await?;

    assert_eq!(session.me_status().await?, 401);
    drop(session);
    server.shutdown().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarted_secret_service_resume_revalidates_tls_and_removes_a_revoked_scoped_token()
-> Result<(), Box<dyn std::error::Error>> {
    let server = atlas_desktop::gate::TlsGateServer::spawn().await?;
    let mut keyring = GateSecretServiceController::start()?;
    let client = GateTransportFactory::from_ca_pem(server.ca_certificate_pem())
        .and_then(|factory| factory.client())?;
    let session = server.login(client).await?;
    let bearer = session.bearer().to_owned();

    run_secret_service_worker(&keyring, "store", server.origin(), &bearer, None, 0)?;
    keyring.restart()?;
    keyring.write_ca(server.ca_certificate_pem())?;

    tokio::task::block_in_place(|| {
        run_secret_service_worker(
            &keyring,
            "resume",
            server.origin(),
            "",
            Some(keyring.ca_path().as_path()),
            0,
        )
    })?;

    session.logout().await?;
    tokio::task::block_in_place(|| {
        run_secret_service_worker(
            &keyring,
            "resume",
            server.origin(),
            "",
            Some(keyring.ca_path().as_path()),
            4,
        )
    })?;
    run_secret_service_worker(&keyring, "load", server.origin(), &bearer, None, 4)?;

    server.shutdown().await?;
    Ok(())
}

fn run_secret_service_worker(
    keyring: &GateSecretServiceController,
    operation: &str,
    origin: &str,
    bearer: &str,
    ca_path: Option<&std::path::Path>,
    expected_status: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut worker = Command::new(env!("CARGO_BIN_EXE_atlas-desktop-secret-service-gate"));
    worker
        .args([operation, origin, "gate-user"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(ca_path) = ca_path {
        worker.env("ATLAS_DESKTOP_GATE_CA_PATH", ca_path);
    }
    keyring.configure_process(&mut worker);

    let mut worker = worker.spawn()?;
    let mut stdin = worker
        .stdin
        .take()
        .ok_or("secret service worker stdin was unavailable")?;
    stdin.write_all(bearer.as_bytes())?;
    drop(stdin);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Some(status) = worker.try_wait()? {
            if status.code() == Some(expected_status) {
                return Ok(());
            }
            return Err(format!("worker returned unexpected status: {status}").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    worker.kill()?;
    worker.wait()?;
    Err("worker did not finish before the bounded timeout".into())
}
