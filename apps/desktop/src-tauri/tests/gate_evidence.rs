#![cfg(feature = "desktop-gate")]

use atlas_desktop::gate::GateEvidenceController;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn evidence_controller_allows_only_named_non_sensitive_fields() {
    let controller = GateEvidenceController::new(["case", "outcome"]);

    let evidence = controller.record([
        ("case", "restart"),
        ("outcome", "restored"),
        ("bearer", "must-not-appear"),
    ]);

    assert_eq!(evidence, "case=restart outcome=restored");
}

#[test]
fn evidence_controller_discards_paths_and_control_characters() {
    let controller = GateEvidenceController::new(["case", "outcome"]);

    let evidence = controller.record([
        ("case", "tls\ninterruption"),
        ("screenshot", "/tmp/secret.png"),
        ("outcome", "recovered\u{7f}"),
    ]);

    assert_eq!(evidence, "case=tlsinterruption outcome=recovered");
}

#[test]
fn evidence_controller_writes_only_sanitized_allowlisted_output()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "atlas-desktop-gate-evidence-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let controller = GateEvidenceController::new(["phase", "outcome"]);

    controller.write(
        &path,
        [
            ("phase", "webdriver-launch"),
            ("outcome", "started"),
            ("bearer", "must-not-appear"),
        ],
    )?;

    assert_eq!(
        fs::read_to_string(&path)?,
        "phase=webdriver-launch outcome=started\n"
    );
    fs::remove_file(path)?;

    Ok(())
}
