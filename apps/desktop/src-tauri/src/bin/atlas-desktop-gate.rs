#[path = "../main.rs"]
mod production_host;

use atlas_desktop::{
    TransportFactory,
    gate::{GateEvidenceController, GateTransportFactory},
};
use std::{env, fs, process};

fn main() {
    let path = match env::var("ATLAS_DESKTOP_GATE_CA_PATH") {
        Ok(path) => path,
        Err(_) => process::exit(2),
    };
    let certificate = match fs::read(path) {
        Ok(certificate) => certificate,
        Err(_) => process::exit(2),
    };
    let client = match GateTransportFactory::from_ca_pem(&certificate)
        .and_then(|factory| factory.client())
    {
        Ok(client) => client,
        Err(_) => process::exit(2),
    };

    if let Ok(path) = env::var("ATLAS_DESKTOP_GATE_EVIDENCE_PATH") {
        let evidence = GateEvidenceController::new(["phase", "outcome"]);
        if evidence
            .write(
                path,
                [("phase", "webdriver-launch"), ("outcome", "started")],
            )
            .is_err()
        {
            process::exit(2);
        }
    }

    production_host::run_with_client(client);
}
