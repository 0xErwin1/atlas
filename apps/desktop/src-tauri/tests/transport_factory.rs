use atlas_desktop::{ReqwestTransportFactory, TransportFactory};

#[test]
fn production_factory_uses_a_client_without_gate_only_roots() {
    let factory = ReqwestTransportFactory::system();

    assert!(factory.client().is_ok());
}

#[cfg(feature = "desktop-gate")]
#[test]
fn gate_factory_accepts_only_a_runtime_supplied_ca() {
    let client = atlas_desktop::gate::GateTransportFactory::from_ca_pem(b"runtime-ca")
        .and_then(|factory| factory.client());

    assert!(client.is_ok());
}
