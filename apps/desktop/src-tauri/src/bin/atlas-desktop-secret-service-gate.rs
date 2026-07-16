use atlas_desktop::{
    DesktopError, DesktopSession, SecretServiceStore, SecretStore, SecretStoreError, SessionScope,
    TransportFactory, gate::GateTransportFactory,
};
use std::{io::Read, process::ExitCode};
use zeroize::Zeroizing;

const UNAVAILABLE: u8 = 4;

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let Some(operation) = arguments.next() else {
        return ExitCode::FAILURE;
    };
    let Some(origin) = arguments.next() else {
        return ExitCode::FAILURE;
    };
    let Some(identity) = arguments.next() else {
        return ExitCode::FAILURE;
    };
    if arguments.next().is_some() {
        return ExitCode::FAILURE;
    }

    let scope = match SessionScope::new(&origin, &identity) {
        Ok(scope) => scope,
        Err(_) => return ExitCode::FAILURE,
    };
    let mut bearer = Zeroizing::new(String::new());
    if std::io::stdin().read_to_string(&mut bearer).is_err() {
        return ExitCode::FAILURE;
    }
    let mut store = SecretServiceStore;

    match operation.as_str() {
        "store" if bearer.is_empty() => ExitCode::FAILURE,
        "store" => finish(store.store(&scope, &bearer)),
        "load" => match store.load(&scope) {
            Ok(value) if !bearer.is_empty() && value == *bearer => ExitCode::SUCCESS,
            Ok(_) => ExitCode::FAILURE,
            Err(error) => finish(Err(error)),
        },
        "remove" => finish(store.remove(&scope)),
        "resume" => resume(&scope),
        _ => ExitCode::FAILURE,
    }
}

fn resume(scope: &SessionScope) -> ExitCode {
    let ca_path = match std::env::var("ATLAS_DESKTOP_GATE_CA_PATH") {
        Ok(path) => path,
        Err(_) => return ExitCode::from(UNAVAILABLE),
    };
    let certificate = match std::fs::read(ca_path) {
        Ok(certificate) => certificate,
        Err(_) => return ExitCode::from(UNAVAILABLE),
    };
    let client = match GateTransportFactory::from_ca_pem(&certificate)
        .and_then(|factory| factory.client())
    {
        Ok(client) => client,
        Err(_) => return ExitCode::from(UNAVAILABLE),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return ExitCode::from(UNAVAILABLE),
    };
    let mut session = DesktopSession::new(SecretServiceStore);
    let result = session.resume_with(scope, |request| {
        let response = runtime
            .block_on(client.execute(request))
            .map_err(|_| DesktopError::TransportUnavailable)?;
        response
            .status()
            .is_success()
            .then_some(())
            .ok_or(DesktopError::SessionInvalid)
    });

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(UNAVAILABLE),
    }
}

fn finish(result: Result<(), SecretStoreError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(SecretStoreError::Unavailable) => ExitCode::from(UNAVAILABLE),
    }
}
