//! Grant-create authority on the Custos authorization routes
//! (`v2-e5-s5b-grant-authority`, GRANT-3/GRANT-4): a non-platform-admin
//! actor may create or revoke a grant only with effective
//! `custos::grant::create`/`delete` on the exact target, granted actions must
//! stay within the actor's effective authority there, credential ceilings
//! apply first (an API key never carries `custos::grant::create`), and a
//! refusal writes a `grant.denied` audit row.
//!
//! This file carries the fixtures the slice needs; the behavioural tests of
//! the non-admin path land once the S6a `AuthorizationService` provides the
//! effective-actions computation. What is pinned already is what holds in
//! every release: a key gets 403 on grant creation regardless of scope.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::authorization::{AuthorityDto, CreateGrantV2Request, SubjectDto, TargetDto};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::ids::ActionId;
use atlas_custos::capability::Capability;
use atlas_custos::entities::authorization::{
    GrantAuthority, GrantId, GrantRecord, NewGrantRecord, SubjectRecord, SubjectSet, TargetRecord,
};
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey, User};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::GrantV2Repo;
use atlas_custos_postgres::repos::authorization::PgGrantV2Repo;
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_server::auth::tokens::{generate_api_key, hash_token};

/// The target every scenario in this file grants on.
const TARGET: &str = "custos::group::g1";

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn target() -> TargetRecord {
    TargetRecord::Ref(TARGET.parse().expect("valid target ref"))
}

/// Seeds a grant of explicit `actions` on [`TARGET`] for `actor`, the way a
/// platform admin would have created it through `POST /grants`.
async fn seed_actor_grant(db: &support::TestDb, actor: &User, actions: &[&str]) -> GrantRecord {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(PrincipalId::from(actor.id)),
        target: target(),
        authority: GrantAuthority::Actions(actions.iter().map(|raw| action(raw)).collect()),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed actor grant")
}

/// An agent API key with `scopes`, owned by a workspace owner and presented
/// as a bearer token.
async fn api_key_client(
    server: &support::TestServer,
    db: &support::TestDb,
    scopes: Vec<Capability>,
) -> AtlasClient {
    let (ws, owner) = support::seed_workspace(db, "grant-authority-key-owner").await;
    let raw_token = generate_api_key();
    let ctx = support::ctx(&ws, &owner);

    PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        atlas_custos::WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "grant-authority-key".to_string(),
            token_hash: hash_token(&raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes,
        },
    )
    .await
    .expect("create api key");

    AtlasClient::new(server.base_url().to_string()).with_token(raw_token)
}

fn status_of(err: ClientError) -> u16 {
    match err {
        ClientError::Api(problem) => problem.status,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn grant_request(actions: &[&str]) -> CreateGrantV2Request {
    CreateGrantV2Request {
        subject: SubjectDto::Principal {
            id: uuid::Uuid::now_v7(),
        },
        target: TargetDto::Ref {
            value: TARGET.to_string(),
        },
        authority: AuthorityDto::Actions {
            actions: actions.iter().map(|raw| (*raw).to_string()).collect(),
        },
    }
}

/// The fixture the non-admin scenarios build on: an editor-like actor whose
/// only authority is `[custos::group::read, custos::grant::create]` on the
/// target, visible through the storage the facts loader will read.
#[tokio::test]
async fn the_seeded_actor_holds_exactly_its_grant_on_the_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, actor) = support::login_user(&server, &db, "grant-authority-actor").await;

    let seeded = seed_actor_grant(
        &db,
        &actor,
        &["custos::group::read", "custos::grant::create"],
    )
    .await;

    let subjects = SubjectSet {
        principal: PrincipalId::from(actor.id),
        groups: vec![],
        principal_sets: vec![],
    };
    let facts = PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .list_for_subjects("custos", &subjects)
    .await
    .expect("list the actor's grants");

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, seeded.id);
    assert_eq!(facts[0].target, target());
    assert_eq!(
        facts[0].authority,
        GrantAuthority::Actions(vec![
            action("custos::group::read"),
            action("custos::grant::create")
        ])
    );

    db.teardown().await;
}

/// GRANT-4 / S7: no key scope translates to `custos::grant::create`, so a
/// key is refused before any grant of its own is consulted, in every release.
#[tokio::test]
async fn an_api_key_with_every_scope_is_403_on_grant_creation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let key = api_key_client(&server, &db, Capability::ALL.to_vec()).await;

    let refused = key
        .custos()
        .create_grant_v2(grant_request(&["custos::group::read"]))
        .await
        .expect_err("a key cannot delegate");
    assert_eq!(status_of(refused), 403);

    db.teardown().await;
}
