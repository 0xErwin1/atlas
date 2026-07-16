#![cfg(feature = "desktop-gate")]

use atlas_desktop::gate::GateController;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

#[tokio::test]
async fn controller_keeps_login_private_and_tears_down_its_ephemeral_resources()
-> Result<(), Box<dyn std::error::Error>> {
    let controller = GateController::start().await?;
    let metadata = controller.metadata();

    assert!(metadata.origin().starts_with("https://localhost:"));
    assert!(!format!("{metadata:?}").contains("password"));
    assert!(!format!("{metadata:?}").contains("token"));
    controller.verify_private_login().await?;

    let teardown = controller.shutdown().await?;

    assert!(!teardown.secret_service_root().exists());
    assert_process_is_gone(teardown.keyring_pid())?;
    assert_process_is_gone(teardown.session_bus_pid())?;
    assert!(teardown.database_dropped());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_drives_the_packaged_vue_webdriver_login_and_restart_flow()
-> Result<(), Box<dyn std::error::Error>> {
    let mut controller = GateController::start().await?;
    let result = controller
        .run_webdriver_login_and_restart(env!("CARGO_BIN_EXE_atlas-desktop-gate"))
        .await?;

    assert!(result.identity_observed());
    assert!(result.protected_rest_verified());
    assert!(result.workspace_event_observed());
    assert_eq!(result.first_workspace_event_count(), 1);
    assert_eq!(result.workspace_event_type(), Some("presence.updated"));
    assert!(result.workspace_event_matches_subscription());
    assert!(result.sse_reconnect_observed());
    assert_eq!(result.recovered_workspace_event_count(), 2);
    assert!(result.auth_remains_valid_after_recovery());
    assert!(result.resumed_without_credentials());
    assert!(result.production_logout_verified());

    controller.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn controller_reaps_webdriver_and_native_children_after_a_failed_start()
-> Result<(), Box<dyn std::error::Error>> {
    let mut controller = GateController::start().await?;

    assert!(
        controller
            .run_webdriver_login_and_restart("/missing/atlas-desktop-gate")
            .await
            .is_err()
    );
    assert!(controller.webdriver_processes_are_stopped());

    controller.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_runs_all_final_cases_and_writes_restricted_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let evidence_path = directory.path().join("gate-evidence.json");
    let mut controller = GateController::start().await?;

    let result = controller
        .run_final_gate(env!("CARGO_BIN_EXE_atlas-desktop-gate"), &evidence_path)
        .await?;

    assert!(result.all_cases_passed());
    assert_eq!(
        fs::metadata(&evidence_path)?.permissions().mode() & 0o777,
        0o600
    );

    let evidence: serde_json::Value = serde_json::from_slice(&fs::read(&evidence_path)?)?;
    assert_eq!(
        evidence
            .pointer("/schema")
            .and_then(serde_json::Value::as_str),
        Some("atlas.desktop.linux-gate-evidence/v1")
    );
    assert_eq!(
        evidence
            .pointer("/cases")
            .and_then(serde_json::Value::as_array)
            .map(|cases| cases.len()),
        Some(7)
    );
    assert!(
        evidence
            .pointer("/cases")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|cases| cases.iter().all(|case| {
                case.get("outcome") == Some(&serde_json::Value::String("pass".to_owned()))
                    && case.as_object().is_some_and(|fields| {
                        fields.keys().all(|key| key == "name" || key == "outcome")
                    })
            }))
    );
    assert!(evidence.get("token").is_none());
    assert!(evidence.get("password").is_none());

    controller.shutdown().await?;
    Ok(())
}

fn assert_process_is_gone(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    assert!(!status.success());
    Ok(())
}
