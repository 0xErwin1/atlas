#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Agent-principal lifecycle routes (`v2-e4-s3a-agents` W4):
//! `POST /agents`, `GET /agents`, `GET /agents/{agent_id}`,
//! `POST /agents/{agent_id}/deactivate`, `POST /agents/{agent_id}/reactivate`.
//!
//! Two authorization facts are load-bearing here and pinned by tests:
//! - Non-disclosure is structural: an agent the caller does not own answers
//!   404 — never a 403 that would confirm the agent exists.
//! - Every route mints, reads, or mutates an agent whose owner is the
//!   calling user (a platform admin bypasses the owner filter for reads and
//!   lifecycle actions, the same `is_root || is_system_admin` definition
//!   `RequireUserAdmin` uses).

mod support;

use atlas_api::dtos::{CreateAgentRequest, CreateUserApiKeyRequest};
use atlas_client::ClientError;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_server::auth::tokens::hash_token;
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey};

fn create_agent_request(display_name: &str) -> CreateAgentRequest {
    CreateAgentRequest {
        display_name: display_name.to_string(),
    }
}

/// Returns the HTTP status of a raw `POST /api/v2/custos/agents` presented
/// with the given bearer token (an API-key principal has no typed-client
/// session to reuse).
async fn create_agent_status_with_bearer(server: &support::TestServer, raw_token: &str) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .post(support::path::api_url(
            server.base_url(),
            "custos",
            "/agents",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .json(&serde_json::json!({ "display_name": "key-authored-agent" }))
        .send()
        .await
        .expect("create-agent request");
    resp.status().as_u16()
}

/// Returns the HTTP status of `GET /api/v2/custos/auth/me` presented with the
/// given raw bearer token.
async fn me_status_with_bearer(server: &support::TestServer, raw_token: &str) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .get(support::path::api_url(
            server.base_url(),
            "custos",
            "/auth/me",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("me request");
    resp.status().as_u16()
}

#[tokio::test]
async fn a_retried_create_agent_with_the_same_idempotency_key_mints_exactly_one_agent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "agents-idempotent").await;
    let token = client.token().expect("session token").to_string();

    let url = support::path::api_url(server.base_url(), "custos", "/agents");
    let http = reqwest::Client::new();
    let body = serde_json::json!({ "display_name": "idempotent-worker" });

    let first = http
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", "agents-idempotency-probe-1")
        .json(&body)
        .send()
        .await
        .expect("first create-agent request");
    assert_eq!(
        first.status().as_u16(),
        201,
        "the first create must succeed"
    );
    let first_body: serde_json::Value = first.json().await.expect("first create body");

    let second = http
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", "agents-idempotency-probe-1")
        .json(&body)
        .send()
        .await
        .expect("retried create-agent request");
    assert_eq!(
        second.status().as_u16(),
        201,
        "a replayed create must return the stored 201"
    );
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|value| value.to_str().ok()),
        Some("true"),
        "the retry must be served from the idempotency store, not re-executed"
    );
    let second_body: serde_json::Value = second.json().await.expect("replayed body");
    assert_eq!(
        second_body["id"], first_body["id"],
        "the replay must return the original agent, not a second one"
    );

    let agents = client.custos().list_agents().await.expect("list agents");
    assert_eq!(
        agents
            .iter()
            .filter(|agent| agent.display_name == "idempotent-worker")
            .count(),
        1,
        "a network retry with the same Idempotency-Key must mint exactly one agent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn creating_an_agent_sets_the_caller_as_its_owner() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "agents-create-owner").await;

    let agent = client
        .custos()
        .create_agent(create_agent_request("my-worker"))
        .await
        .expect("create agent");

    assert_eq!(agent.display_name, "my-worker");
    assert_eq!(
        agent.owner, user.id.0,
        "POST /agents must record the calling user as the agent's owner"
    );
    assert!(
        agent.deactivated_at.is_none(),
        "a freshly created agent must be active"
    );

    let fetched = client
        .custos()
        .get_agent(agent.id)
        .await
        .expect("fetch own agent");
    assert_eq!(fetched.id, agent.id);
    assert_eq!(fetched.owner, user.id.0);

    db.teardown().await;
}

#[tokio::test]
async fn listing_agents_shows_only_the_callers_own_agents() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, _user_a) = support::login_user(&server, &db, "agents-list-a").await;
    let (client_b, _user_b) = support::login_user(&server, &db, "agents-list-b").await;

    let agent_a = client_a
        .custos()
        .create_agent(create_agent_request("a-worker"))
        .await
        .expect("create agent for a");
    let _agent_b = client_b
        .custos()
        .create_agent(create_agent_request("b-worker"))
        .await
        .expect("create agent for b");

    let list_a = client_a.custos().list_agents().await.expect("list for a");
    assert_eq!(
        list_a.iter().filter(|agent| agent.id == agent_a.id).count(),
        1,
        "the caller's own agent must appear in their list"
    );
    assert!(
        list_a.iter().all(|agent| agent.display_name != "b-worker"),
        "another user's agent must not appear in the caller's list"
    );

    let list_b = client_b.custos().list_agents().await.expect("list for b");
    assert!(
        list_b.iter().all(|agent| agent.display_name != "a-worker"),
        "the owner-scoped list must not leak the other caller's agent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn a_platform_admin_sees_every_agent_in_the_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, _user_a) = support::login_user(&server, &db, "agents-admin-a").await;
    let (client_b, _user_b) = support::login_user(&server, &db, "agents-admin-b").await;

    client_a
        .custos()
        .create_agent(create_agent_request("admin-view-a"))
        .await
        .expect("create agent for a");
    client_b
        .custos()
        .create_agent(create_agent_request("admin-view-b"))
        .await
        .expect("create agent for b");

    let root = support::login_root_user(&server, &db).await;
    let all = root.custos().list_agents().await.expect("admin list");

    assert!(
        all.iter().any(|agent| agent.display_name == "admin-view-a"),
        "a platform admin must see user a's agent"
    );
    assert!(
        all.iter().any(|agent| agent.display_name == "admin-view-b"),
        "a platform admin must see user b's agent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn a_foreign_agent_answers_404_not_403() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, _user_a) = support::login_user(&server, &db, "agents-foreign-a").await;
    let (client_b, _user_b) = support::login_user(&server, &db, "agents-foreign-b").await;

    let agent_b = client_b
        .custos()
        .create_agent(create_agent_request("b-private"))
        .await
        .expect("create agent for b");

    let not_found = |error: ClientError, action: &str| {
        matches!(error, ClientError::Api(ref problem) if problem.status == 404)
            .then_some(())
            .unwrap_or_else(|| {
                panic!("{action} must answer 404 for a foreign agent, got {error:?}")
            })
    };

    not_found(
        client_a.custos().get_agent(agent_b.id).await.unwrap_err(),
        "GET /agents/{id}",
    );
    not_found(
        client_a
            .custos()
            .deactivate_agent(agent_b.id)
            .await
            .unwrap_err(),
        "POST /agents/{id}/deactivate",
    );
    not_found(
        client_a
            .custos()
            .reactivate_agent(agent_b.id)
            .await
            .unwrap_err(),
        "POST /agents/{id}/reactivate",
    );
    not_found(
        client_a
            .custos()
            .get_agent(uuid::Uuid::now_v7())
            .await
            .unwrap_err(),
        "GET /agents/{id} with a nonexistent id",
    );

    db.teardown().await;
}

#[tokio::test]
async fn deactivate_and_reactivate_round_trip_restores_an_agents_key() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "agents-roundtrip").await;

    // Mint an agent credential the way the app does: an agent api key mints
    // its own agent principal, owned by the key's creator, so the principal
    // is reachable through the same /agents routes this slice adds.
    let raw_token = "atlas_ak_roundtripBody0123456789abcdefghij00";
    db.api_key_repo()
        .create_for_user(
            user.id,
            NewApiKey {
                name: "roundtrip-agent".to_string(),
                token_hash: hash_token(raw_token),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: Vec::new(),
            },
        )
        .await
        .expect("create agent api key");

    // The key's agent principal is visible in the owner's agent list. The
    // entity does not carry the principal id, but the minted principal's
    // display name is the key's name.
    let listed = client.custos().list_agents().await.expect("list agents");
    let agent = listed
        .iter()
        .find(|agent| agent.display_name == "roundtrip-agent")
        .unwrap_or_else(|| panic!("the key's agent principal must appear in its owner's list"))
        .clone();

    assert_eq!(
        me_status_with_bearer(&server, raw_token).await,
        200,
        "the agent key must authenticate before deactivation"
    );

    let deactivated = client
        .custos()
        .deactivate_agent(agent.id)
        .await
        .expect("deactivate own agent");
    assert!(
        deactivated.deactivated_at.is_some(),
        "deactivation must stamp deactivated_at"
    );

    assert_eq!(
        me_status_with_bearer(&server, raw_token).await,
        401,
        "a deactivated agent principal's key must stop authenticating"
    );

    let reactivated = client
        .custos()
        .reactivate_agent(agent.id)
        .await
        .expect("reactivate own agent");
    assert!(
        reactivated.deactivated_at.is_none(),
        "reactivation must clear deactivated_at"
    );

    assert_eq!(
        me_status_with_bearer(&server, raw_token).await,
        200,
        "reactivating the agent principal must restore its key's authentication"
    );

    db.teardown().await;
}

#[tokio::test]
async fn an_api_key_principal_cannot_manage_agents() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "agents-key-caller").await;

    let created = client
        .custos()
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "agents-key-caller-key".to_string(),
            r#type: Some("agent".to_string()),
            key_kind: Some("agent".to_string()),
            expires_at: None,
            scopes: None,
            initial_grant: None,
        })
        .await
        .expect("create agent api key");
    drop(client);

    let status = create_agent_status_with_bearer(&server, &created.secret).await;
    assert_eq!(
        status, 403,
        "an agent (API-key) principal must not manage agents: agents cannot \
         mint or own other agents"
    );

    db.teardown().await;
}
