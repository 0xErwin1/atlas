#![allow(clippy::expect_used)]

use atlas_desktop::{
    DesktopApiRequest, DesktopConfiguration, DesktopSession, InMemorySecretStore, Lifecycle,
    LifecycleAction, SecretStore, SessionScope, SessionState, StreamFrame, StreamTermination,
    TransportKind, WorkspaceEvent, build_authenticated_api_request, build_authenticated_request,
    classify_workspace_stream_terminal, process_workspace_sse_chunk,
};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

const ORIGIN: &str = "https://atlas.example.test";
const BEARER: &str = "test-bearer-material";

#[test]
fn rejects_noncanonical_or_non_https_origins() {
    for origin in [
        "http://atlas.example.test",
        "https://ATLAS.example.test",
        "https://user:password@atlas.example.test",
        "https://atlas.example.test/path",
        "https://atlas.example.test:443",
        "https://_bad.example.test",
    ] {
        assert!(
            SessionScope::new(origin, "user-1").is_err(),
            "{origin} must be rejected"
        );
    }

    assert_eq!(
        SessionScope::new(ORIGIN, "user-1").map(|scope| scope.origin().to_owned()),
        Ok(ORIGIN.to_owned())
    );
}

#[test]
fn fails_closed_when_keyring_material_is_missing_or_locked() {
    let scope = SessionScope::new(ORIGIN, "user-1").expect("valid scope");
    let mut missing = Lifecycle::new(InMemorySecretStore::missing());
    let mut locked = Lifecycle::new(InMemorySecretStore::locked());

    assert_eq!(missing.resume(&scope), SessionState::Unauthenticated);
    assert_eq!(locked.resume(&scope), SessionState::Unauthenticated);
    assert_eq!(
        missing.take_action(),
        Some(LifecycleAction::PurgeScopedCache(scope.clone()))
    );
    assert_eq!(
        locked.take_action(),
        Some(LifecycleAction::PurgeScopedCache(scope))
    );
}

#[test]
fn secret_store_read_and_delete_failures_purge_only_the_affected_scope() {
    let first = SessionScope::new(ORIGIN, "user-1").expect("first scope");
    let second = SessionScope::new("https://other.example.test", "user-2").expect("second scope");
    let mut lifecycle = Lifecycle::new(InMemorySecretStore::locked());

    assert_eq!(lifecycle.resume(&first), SessionState::Unauthenticated);
    assert_eq!(
        lifecycle.take_action(),
        Some(LifecycleAction::PurgeScopedCache(first.clone()))
    );

    lifecycle.expire_or_revoke(&first);
    assert_eq!(
        lifecycle.take_action(),
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(first))
    );
    assert_eq!(lifecycle.resume(&second), SessionState::Unauthenticated);
    assert_eq!(
        lifecycle.take_action(),
        Some(LifecycleAction::PurgeScopedCache(second))
    );
}

#[test]
fn stores_bearer_material_in_the_secret_store_before_restart_resume() {
    let scope = SessionScope::new(ORIGIN, "user-1").expect("valid scope");
    let mut lifecycle = Lifecycle::new(InMemorySecretStore::missing());

    assert_eq!(
        lifecycle.store_session(&scope, BEARER),
        SessionState::Authenticated
    );
    assert!(lifecycle.transport_active());
    assert_eq!(lifecycle.resume(&scope), SessionState::Authenticated);
    assert_eq!(lifecycle.take_action(), None);
}

#[test]
fn constructs_bearer_auth_for_rest_and_sse_without_exposing_the_token() {
    let rest =
        build_authenticated_request(ORIGIN, "GET", "/api/auth/me", BEARER, TransportKind::Rest)
            .expect("valid REST request");
    let sse = build_authenticated_request(
        ORIGIN,
        "GET",
        "/api/workspaces/atlas/events",
        BEARER,
        TransportKind::Sse,
    )
    .expect("valid SSE request");

    assert_eq!(
        rest.url().as_str(),
        "https://atlas.example.test/api/auth/me"
    );
    assert_eq!(
        rest.headers()["authorization"],
        "Bearer test-bearer-material"
    );
    assert_eq!(sse.headers()["accept"], "text/event-stream");
    assert_eq!(
        sse.headers()["authorization"],
        "Bearer test-bearer-material"
    );
    assert!(
        build_authenticated_request(
            ORIGIN,
            "GET",
            "https://other.example/api",
            BEARER,
            TransportKind::Rest
        )
        .is_err()
    );
}

#[test]
fn rejects_non_allowlisted_methods_and_api_path_escape_attempts() {
    for method in ["TRACE", "CONNECT"] {
        assert!(
            build_authenticated_request(
                ORIGIN,
                method,
                "/api/auth/me",
                BEARER,
                TransportKind::Rest
            )
            .is_err(),
            "{method} must not receive a bearer"
        );
    }

    for path in [
        "/api/../admin",
        "/api/%2e%2e/admin",
        "/api/%2E%2E/admin",
        "/api\\..\\admin",
        "/api/%5c..%5cadmin",
        "/api/auth/me%0a",
        "//atlas.example.test/api/auth/me",
        "https://other.example/api/auth/me",
    ] {
        assert!(
            build_authenticated_request(ORIGIN, "GET", path, BEARER, TransportKind::Rest).is_err(),
            "{path} must not receive a bearer"
        );
    }
}

#[test]
fn generic_desktop_api_request_preserves_method_query_headers_and_body_beneath_api_root() {
    let body = br#"{"title":"Draft"}"#.to_vec();
    let request = build_authenticated_api_request(
        ORIGIN,
        BEARER,
        DesktopApiRequest {
            method: "PATCH".to_owned(),
            path: "/api/workspaces/acme/tasks/ATL-1?detail=full".to_owned(),
            headers: vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-atlas-csrf".to_owned(), "1".to_owned()),
            ],
            body: body.clone(),
        },
    )
    .expect("the generated-client request is valid");

    assert_eq!(request.method(), "PATCH");
    assert_eq!(
        request.url().as_str(),
        "https://atlas.example.test/api/workspaces/acme/tasks/ATL-1?detail=full"
    );
    assert_eq!(request.headers()["content-type"], "application/json");
    assert_eq!(request.headers()["x-atlas-csrf"], "1");
    assert_eq!(
        request.headers()["authorization"],
        "Bearer test-bearer-material"
    );
    assert_eq!(
        request.body().and_then(reqwest::Body::as_bytes),
        Some(body.as_slice())
    );
}

#[test]
fn generic_desktop_api_request_rejects_authorization_overrides_and_non_api_urls() {
    for request in [
        DesktopApiRequest {
            method: "GET".to_owned(),
            path: "https://other.example/api/workspaces".to_owned(),
            headers: vec![],
            body: vec![],
        },
        DesktopApiRequest {
            method: "GET".to_owned(),
            path: "/api/workspaces".to_owned(),
            headers: vec![("authorization".to_owned(), "Bearer attacker".to_owned())],
            body: vec![],
        },
    ] {
        assert!(build_authenticated_api_request(ORIGIN, BEARER, request).is_err());
    }
}

#[test]
fn aligns_canonical_ipv4_and_ipv6_origins_with_the_gate_harness() {
    assert!(SessionScope::new("https://127.000.000.001", "user-1").is_err());
    assert!(SessionScope::new("https://[2001:db8::1]", "user-1").is_ok());
    assert!(SessionScope::new("https://[2001:0db8::1]", "user-1").is_err());
}

#[test]
fn accepts_canonical_non_default_ports_but_rejects_explicit_default_ports() {
    assert_eq!(
        SessionScope::new("https://atlas.example.test:8443", "user-1")
            .map(|scope| scope.origin().to_owned()),
        Ok("https://atlas.example.test:8443".to_owned())
    );
    assert!(SessionScope::new("https://atlas.example.test:443", "user-1").is_err());
    assert!(SessionScope::new("https://[2001:db8::1]:8443", "user-1").is_ok());
}

#[test]
fn normalizes_a_trailing_slash_and_derives_all_requests_from_the_api_service_root() {
    let scope = SessionScope::new("https://atlas.iperez.dev/", "user-1")
        .expect("a configured trailing slash is normalized");
    let request = build_authenticated_request(
        scope.origin(),
        "GET",
        "/api/workspaces/atlas/events",
        BEARER,
        TransportKind::Sse,
    )
    .expect("the normalized custom origin keeps the request beneath its service root");

    assert_eq!(scope.origin(), "https://atlas.iperez.dev");
    assert_eq!(
        request.url().as_str(),
        "https://atlas.iperez.dev/api/workspaces/atlas/events"
    );
}

#[test]
fn persists_only_the_normalized_user_selected_origin_in_desktop_configuration() {
    let directory = std::env::temp_dir().join(format!(
        "atlas-desktop-origin-contract-{}",
        std::process::id()
    ));
    let configuration =
        DesktopConfiguration::from_selected_origin("https://atlas.iperez.dev:8443/")
            .expect("a configured trailing slash is normalized");

    configuration
        .save(&directory)
        .expect("the selected origin is persisted");

    let configuration_file = directory.join("desktop.json");
    let persisted = fs::read_to_string(&configuration_file).expect("configuration is readable");
    let loaded = DesktopConfiguration::load(&directory).expect("configuration reloads");

    assert_eq!(
        persisted,
        "{\"origin\":\"https://atlas.iperez.dev:8443\"}\n"
    );
    assert_eq!(loaded.origin(), "https://atlas.iperez.dev:8443");
    assert!(!persisted.contains("bearer"));

    fs::remove_dir_all(directory).expect("temporary configuration is removed");
}

#[test]
fn session_revalidation_and_realtime_use_the_scoped_workspace_endpoint() {
    let scope = SessionScope::new("https://atlas.example.test:8443", "user-1")
        .expect("canonical non-default origin");
    let mut session = DesktopSession::new(InMemorySecretStore::with_session(scope.clone(), BEARER));

    session
        .resume_with(&scope, |request| {
            assert_eq!(
                request.url().as_str(),
                "https://atlas.example.test:8443/api/auth/me"
            );
            Ok(())
        })
        .expect("stored session must revalidate through the transport");

    let event = session
        .connect_workspace_events(&scope, "atlas", |request| {
            assert_eq!(
                request.url().as_str(),
                "https://atlas.example.test:8443/api/workspaces/atlas/events"
            );
            Ok("event: task.updated\ndata: {\"id\":\"event-1\",\"event_type\":\"task.updated\",\"version\":1,\"source\":\"server\",\"workspace_id\":\"workspace-1\",\"occurred_at\":\"2026-01-01T00:00:00Z\",\"actor\":{\"type\":\"user\",\"id\":\"user-1\"},\"data\":{\"task_id\":\"task-1\"}}\n\n".to_owned())
        })
        .expect("SSE transport must execute the workspace request");

    assert_eq!(
        event,
        WorkspaceEvent {
            event_type: "task.updated".to_owned(),
            data: serde_json::json!({"task_id": "task-1"}),
        }
    );
}

#[test]
fn failed_restart_revalidation_removes_only_the_invalid_origin_session() {
    let first = SessionScope::new("https://atlas.example.test:8443", "user-1")
        .expect("canonical non-default origin");
    let second =
        SessionScope::new("https://other.example.test", "user-2").expect("unrelated origin");
    let mut store = InMemorySecretStore::with_session(first.clone(), BEARER);
    store
        .store(&second, "other-bearer-material")
        .expect("second session stored");
    let mut session = DesktopSession::new(store);

    assert_eq!(
        session.resume_with(&first, |request| {
            assert_eq!(
                request.url().as_str(),
                "https://atlas.example.test:8443/api/auth/me"
            );
            Err::<(), _>(atlas_desktop::DesktopError::SessionInvalid)
        }),
        Err(atlas_desktop::DesktopError::SessionInvalid)
    );
    assert_eq!(
        session.take_action(),
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(
            first.clone()
        ))
    );
    assert!(
        session
            .authenticated_request(&first, "/api/auth/me", TransportKind::Rest)
            .is_err()
    );
    assert!(
        session
            .authenticated_request(&second, "/api/auth/me", TransportKind::Rest)
            .is_ok()
    );
}

#[test]
fn transient_rest_revalidation_preserves_the_valid_scoped_session() {
    let first = SessionScope::new("https://atlas.example.test:8443", "user-1")
        .expect("canonical non-default origin");
    let second =
        SessionScope::new("https://other.example.test", "user-2").expect("unrelated origin");
    let mut store = InMemorySecretStore::with_session(first.clone(), BEARER);
    store
        .store(&second, "other-bearer-material")
        .expect("second session stored");
    let mut session = DesktopSession::new(store);

    assert_eq!(
        session.resume_with(&first, |_| Err::<(), _>(
            atlas_desktop::DesktopError::TransportUnavailable
        )),
        Err(atlas_desktop::DesktopError::TransportUnavailable)
    );
    assert_eq!(session.take_action(), None);
    assert!(
        session
            .authenticated_request(&first, "/api/auth/me", TransportKind::Rest)
            .is_ok()
    );
    assert!(
        session
            .authenticated_request(&second, "/api/auth/me", TransportKind::Rest)
            .is_ok()
    );
}

#[test]
fn transient_workspace_stream_failure_preserves_the_valid_scoped_session() {
    let scope = SessionScope::new("https://atlas.example.test:8443", "user-1")
        .expect("canonical non-default origin");
    let mut session = DesktopSession::new(InMemorySecretStore::with_session(scope.clone(), BEARER));

    assert_eq!(
        session.connect_workspace_events(&scope, "atlas", |_| {
            Err::<String, _>(atlas_desktop::DesktopError::TransportUnavailable)
        }),
        Err(atlas_desktop::DesktopError::TransportUnavailable)
    );
    assert_eq!(session.take_action(), None);
    assert!(
        session
            .authenticated_request(&scope, "/api/auth/me", TransportKind::Rest)
            .is_ok()
    );
}

#[test]
fn injected_transport_executes_local_rest_and_sse_boundaries() {
    let scope = SessionScope::new(ORIGIN, "user-1").expect("valid scope");
    let mut session = DesktopSession::new(InMemorySecretStore::with_session(scope.clone(), BEARER));
    let rest = spawn_fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");

    session
        .resume_with(&scope, |request| {
            execute_fixture(rest, request.url().path(), false).map(|_| ())
        })
        .expect("revalidation must execute against the fixture");

    let sse = spawn_fixture(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 182\r\n\r\nevent: task.updated\ndata: {\"id\":\"event-1\",\"event_type\":\"task.updated\",\"version\":1,\"source\":\"server\",\"workspace_id\":\"workspace-1\",\"occurred_at\":\"2026-01-01T00:00:00Z\",\"actor\":{\"type\":\"user\",\"id\":\"user-1\"},\"data\":{\"task_id\":\"task-1\"}}\n\n",
    );
    let event = session
        .connect_workspace_events(&scope, "atlas", |request| {
            execute_fixture(sse, request.url().path(), true)
        })
        .expect("workspace SSE must execute against the fixture");

    assert_eq!(event.event_type, "task.updated");
    assert_eq!(event.data, serde_json::json!({"task_id": "task-1"}));
}

#[test]
fn production_sse_parser_emits_a_live_envelope_once_without_wrapping_it_again() {
    let envelope = serde_json::json!({
        "id": "event-1",
        "event_type": "task.updated",
        "version": 1,
        "source": "server",
        "workspace_id": "workspace-1",
        "occurred_at": "2026-01-01T00:00:00Z",
        "actor": {"type": "user", "id": "user-1"},
        "data": {"task_id": "task-1"}
    });
    let mut pending = String::new();
    let mut emitted = Vec::new();
    let frame = format!("event: task.updated\ndata: {envelope}\n\n");

    process_workspace_sse_chunk(&mut pending, frame.as_bytes(), |frame| {
        emitted.push(frame);
        Ok(())
    })
    .expect("a valid Atlas LiveEnvelope frame is emitted");

    assert_eq!(emitted, vec![StreamFrame::LiveEnvelope(envelope)]);
}

#[test]
fn production_sse_parser_preserves_server_resync_without_treating_it_as_auth_loss() {
    let mut pending = String::new();
    let mut emitted = Vec::new();

    process_workspace_sse_chunk(&mut pending, b"event: resync\n\n", |frame| {
        emitted.push(frame);
        Ok(())
    })
    .expect("a server resync frame is valid");

    assert_eq!(emitted, vec![StreamFrame::Resync]);
}

#[test]
fn stream_terminal_classification_preserves_auth_for_eof_and_transient_failures() {
    assert_eq!(
        classify_workspace_stream_terminal(None),
        StreamTermination::Reconnect
    );
    assert_eq!(
        classify_workspace_stream_terminal(Some(500)),
        StreamTermination::Reconnect
    );
    assert_eq!(
        classify_workspace_stream_terminal(Some(401)),
        StreamTermination::AuthLoss
    );
}

fn spawn_fixture(response: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds a local port");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("fixture accepts one request");
        let mut request = [0_u8; 2048];
        let received = stream.read(&mut request).expect("fixture reads request");
        let request = std::str::from_utf8(
            request
                .get(..received)
                .expect("read length fits the request buffer"),
        )
        .expect("request is HTTP text");
        assert!(request.contains("Authorization: Bearer test-bearer-material"));
        stream
            .write_all(response.as_bytes())
            .expect("fixture writes response");
    });
    port
}

fn execute_fixture(
    port: u16,
    path: &str,
    sse: bool,
) -> Result<String, atlas_desktop::DesktopError> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|_| atlas_desktop::DesktopError::TransportUnavailable)?;
    let accept = if sse {
        "text/event-stream"
    } else {
        "application/json"
    };
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: atlas.example.test\r\nAuthorization: Bearer {BEARER}\r\nAccept: {accept}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| atlas_desktop::DesktopError::TransportUnavailable)?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| atlas_desktop::DesktopError::TransportUnavailable)?;
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .ok_or(atlas_desktop::DesktopError::TransportUnavailable)
}

#[test]
fn revoked_scope_cancels_only_its_transport_and_emits_one_scoped_purge() {
    let first = SessionScope::new(ORIGIN, "user-1").expect("valid first scope");
    let second =
        SessionScope::new("https://other.example.test", "user-2").expect("valid second scope");
    let mut store = InMemorySecretStore::with_session(first.clone(), BEARER);
    store
        .store(&second, "other-bearer-material")
        .expect("second scope stored");
    let mut session = DesktopSession::new(store);

    session
        .resume_with(&first, |_| Ok(()))
        .expect("first scope resumes");
    session
        .resume_with(&second, |_| Ok(()))
        .expect("second scope resumes");

    assert_eq!(
        session.revoke(&first),
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(
            first.clone()
        ))
    );
    assert!(session.transport_is_cancelled(&first));
    assert!(!session.transport_is_cancelled(&second));
    assert_eq!(session.take_action(), None);
}

#[test]
fn failed_remote_logout_still_removes_only_the_affected_session_and_requests_a_scoped_purge() {
    let first = SessionScope::new(ORIGIN, "user-1").expect("valid first scope");
    let second =
        SessionScope::new("https://other.example.test", "user-2").expect("valid second scope");
    let mut store = InMemorySecretStore::with_session(first.clone(), BEARER);
    store
        .store(&second, "other-bearer-material")
        .expect("second session stored");
    let mut session = DesktopSession::new(store);

    let outcome = session.logout_with(&first, |request| {
        assert_eq!(request.method(), "POST");
        assert_eq!(
            request.url().as_str(),
            "https://atlas.example.test/api/auth/logout"
        );

        Err(atlas_desktop::DesktopError::TransportUnavailable)
    });

    assert_eq!(
        outcome.remote_result,
        Err(atlas_desktop::DesktopError::TransportUnavailable)
    );
    assert_eq!(
        outcome.action,
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(
            first.clone()
        ))
    );
    assert!(
        session
            .authenticated_request(&first, "/api/auth/me", TransportKind::Rest)
            .is_err()
    );
    assert!(
        session
            .authenticated_request(&second, "/api/auth/me", TransportKind::Rest)
            .is_ok()
    );
}

#[test]
fn restart_returns_the_stored_bearer_for_revalidation() {
    let scope = SessionScope::new(ORIGIN, "user-1").expect("valid scope");
    let store = InMemorySecretStore::with_session(scope.clone(), BEARER);

    assert_eq!(store.load(&scope), Ok(BEARER.to_owned()));
}

#[test]
fn expiry_logout_and_restart_cancel_transport_and_purge_only_the_session_scope() {
    let scope = SessionScope::new(ORIGIN, "user-1").expect("valid scope");
    let mut store = InMemorySecretStore::with_session(scope.clone(), BEARER);
    let mut lifecycle = Lifecycle::new(store.clone());

    assert_eq!(lifecycle.resume(&scope), SessionState::Authenticated);
    assert!(lifecycle.transport_active());

    lifecycle.expire_or_revoke(&scope);
    assert!(!lifecycle.transport_active());
    assert_eq!(
        lifecycle.take_action(),
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(
            scope.clone()
        ))
    );

    store.remove(&scope);
    let mut restarted = Lifecycle::new(store);
    assert_eq!(restarted.resume(&scope), SessionState::Unauthenticated);
    assert_eq!(
        restarted.take_action(),
        Some(LifecycleAction::PurgeScopedCache(scope))
    );
}

#[test]
fn lifecycle_purge_action_is_scoped_to_the_expired_identity() {
    let first = SessionScope::new(ORIGIN, "user-1").expect("valid first scope");
    let second =
        SessionScope::new("https://other.example.test", "user-2").expect("valid second scope");
    let mut store = InMemorySecretStore::with_session(first.clone(), BEARER);
    store
        .store(&second, "other-bearer-material")
        .expect("second session stored");
    let mut lifecycle = Lifecycle::new(store.clone());

    lifecycle.expire_or_revoke(&first);

    assert_eq!(
        lifecycle.take_action(),
        Some(LifecycleAction::CancelTransportAndPurgeScopedCache(first))
    );
    assert_eq!(store.load(&second), Ok("other-bearer-material".to_owned()));
}
