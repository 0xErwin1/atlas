use atlas_desktop::gate::GateController;
use std::{env, path::PathBuf, process};

#[tokio::main]
async fn main() {
    let mut arguments = env::args().skip(1);
    let application = match (arguments.next().as_deref(), arguments.next()) {
        (Some("--application"), Some(application)) => application,
        _ => process::exit(2),
    };
    let evidence = match (
        arguments.next().as_deref(),
        arguments.next(),
        arguments.next(),
    ) {
        (Some("--evidence"), Some(evidence), None) => PathBuf::from(evidence),
        _ => process::exit(2),
    };

    let mut controller = match GateController::start().await {
        Ok(controller) => controller,
        Err(_) => process::exit(1),
    };
    let result = controller.run_final_gate(&application, &evidence).await;
    let teardown = controller.shutdown().await;

    if result.is_err() || teardown.is_err() {
        process::exit(1);
    }
}
