#![cfg(feature = "desktop-gate")]

use atlas_desktop::gate::GateSecretServiceController;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};

const FIRST_ORIGIN: &str = "https://first.example";
const SECOND_ORIGIN: &str = "https://second.example";
const IDENTITY: &str = "gate-user";

#[test]
fn isolated_secret_service_survives_restart_and_contains_failure_states()
-> Result<(), Box<dyn std::error::Error>> {
    let mut controller = GateSecretServiceController::start()?;

    run_worker(&controller, "store", FIRST_ORIGIN, "first-bearer", 0)?;
    run_worker(&controller, "store", SECOND_ORIGIN, "second-bearer", 0)?;
    run_worker(&controller, "load", FIRST_ORIGIN, "first-bearer", 0)?;
    run_worker(&controller, "load", SECOND_ORIGIN, "second-bearer", 0)?;

    controller.restart()?;
    run_worker(&controller, "load", FIRST_ORIGIN, "first-bearer", 0)?;
    run_worker(&controller, "load", SECOND_ORIGIN, "second-bearer", 0)?;

    run_worker(&controller, "remove", FIRST_ORIGIN, "first-bearer", 0)?;
    run_worker(&controller, "load", FIRST_ORIGIN, "first-bearer", 4)?;
    run_worker(&controller, "load", SECOND_ORIGIN, "second-bearer", 0)?;

    controller.lock_default_collection()?;
    run_worker(&controller, "store", FIRST_ORIGIN, "replacement-bearer", 4)?;

    controller.stop()?;
    run_worker(&controller, "load", SECOND_ORIGIN, "second-bearer", 4)?;

    Ok(())
}

fn run_worker(
    controller: &GateSecretServiceController,
    operation: &str,
    origin: &str,
    bearer: &str,
    expected_status: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut worker = Command::new(env!("CARGO_BIN_EXE_atlas-desktop-secret-service-gate"));
    worker
        .args([operation, origin, IDENTITY])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    controller.configure_process(&mut worker);

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
