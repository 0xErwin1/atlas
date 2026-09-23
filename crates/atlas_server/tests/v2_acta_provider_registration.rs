//! Acta's resource provider inside the server's V2 authorization service
//! (`v2-e7-s1b-acta-provider`, ACTA-AUTHZ-1/5): the composed service answers
//! Acta targets from Acta's own rows, and a V2 grant on a live document
//! allows while the same document in the trash is not found.
//!
//! The registry's Acta declaration publishes no V2 kinds yet, so the grant
//! test validates against a catalog carrying an explicit Acta product spec.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use atlas_acta::provider::ActaResourceProvider;
use atlas_acta_postgres::repos::resource_store::PgActaResourceStore;
use atlas_core::ids::{ActionId, ResourceRef};
use atlas_core::registry::ComponentId;
use atlas_custos::authorize::{
    ActorContext, AuthorizationService, AuthorizationSettings, ProviderSet,
};
use atlas_custos::entities::authorization::{
    GrantAuthority, GrantId, NewGrantRecord, SubjectRecord, TargetRecord,
};
use atlas_custos::eval::{Catalog, Ceiling, Decision, DenyMode, ProductSpec};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::GrantV2Repo;
use atlas_custos_postgres::repos::authorization::PgGrantV2Repo;
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_server::authz::v2_service::{TokioSleeper, build_authorization_service, product_specs};
use atlas_server::config::DenyModeConfig;
use atlas_server::state::AppState;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

const DOCUMENT_READ: &str = "acta::document::read";

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn document_ref(id: Uuid) -> ResourceRef {
    format!("acta::document::{id}")
        .parse()
        .expect("valid document ref")
}

fn actor(principal: PrincipalId, is_root: bool) -> ActorContext {
    ActorContext {
        principal,
        is_root,
        ceiling: Ceiling::Unrestricted,
    }
}

/// A live document at the root of a fresh workspace, and its creator.
async fn seed_document(db: &support::TestDb, name: &str) -> (Uuid, PrincipalId) {
    let (ws, owner) = support::seed_workspace(db, name).await;
    let document = Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO acta.documents (id, workspace_id, title, created_by_user_id) \
             VALUES ('{document}', '{}', 'Doc', '{}')",
            ws.id.0, owner.id.0
        ))
        .await
        .expect("seed document");

    (document, PrincipalId::from(owner.id))
}

async fn trash_document(db: &support::TestDb, document: Uuid) {
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE acta.documents SET deleted_at = now() WHERE id = '{document}'"
        ))
        .await
        .expect("trash document");
}

/// The registry's catalog with an explicit Acta product spec in place of
/// the registry's own Acta declaration.
fn catalog_with_acta(state: &AppState) -> Catalog {
    let mut specs: Vec<ProductSpec> = product_specs(&state.registry)
        .into_iter()
        .filter(|spec| spec.product != "acta")
        .collect();
    specs.push(ProductSpec {
        product: "acta".to_string(),
        kinds: vec!["workspace".to_string(), "document".to_string()],
        actions: vec![action(DOCUMENT_READ)],
        roles: vec![],
        principal_sets: vec!["members".to_string()],
    });

    Catalog::new(specs).expect("catalog with acta")
}

#[tokio::test]
async fn the_composed_service_answers_acta_targets_from_acta_rows() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let service = build_authorization_service(
        &state.registry,
        db.conn().clone(),
        DenyModeConfig::Disabled,
        Duration::from_secs(5),
    )
    .expect("authorization service");
    let (document, owner) = seed_document(&db, "acta-registration").await;
    let root = actor(owner, true);

    let live = service
        .authorize(&root, &action(DOCUMENT_READ), &document_ref(document))
        .await
        .expect("acta facts are available");
    trash_document(&db, document).await;
    let trashed = service
        .authorize(&root, &action(DOCUMENT_READ), &document_ref(document))
        .await
        .expect("acta facts are available");

    assert_eq!(live.decision, Decision::Allowed);
    assert_eq!(trashed.decision, Decision::NotFound);
}

#[tokio::test]
async fn a_v2_grant_on_an_acta_document_allows_until_the_document_is_trashed() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let acta = ComponentId::new("acta")
        .ok()
        .and_then(|id| state.registry.get(&id))
        .expect("the registry declares acta");
    let service = AuthorizationService::new(
        ProviderSet::new().with(
            "acta",
            Arc::new(ActaResourceProvider::new(
                PgActaResourceStore {
                    conn: db.conn().clone(),
                },
                &acta.authorization,
            )),
        ),
        PgAuthorizationFactsStore {
            conn: db.conn().clone(),
        },
        PgGroupMembershipSource {
            conn: db.conn().clone(),
        },
        Arc::new(TokioSleeper),
        AuthorizationSettings {
            catalog: catalog_with_acta(&state),
            deny_mode: DenyMode::Disabled,
            provider_timeout: Duration::from_secs(5),
        },
    );
    let (document, owner) = seed_document(&db, "acta-grant").await;
    let (other, _) = seed_document(&db, "acta-grant-other").await;
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(owner),
        target: TargetRecord::Ref(document_ref(document)),
        authority: GrantAuthority::Actions(vec![action(DOCUMENT_READ)]),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed grant");
    let grantee = actor(owner, false);

    let granted = service
        .authorize(&grantee, &action(DOCUMENT_READ), &document_ref(document))
        .await
        .expect("granted document");
    let ungranted = service
        .authorize(&grantee, &action(DOCUMENT_READ), &document_ref(other))
        .await
        .expect("ungranted document");
    trash_document(&db, document).await;
    let trashed = service
        .authorize(&grantee, &action(DOCUMENT_READ), &document_ref(document))
        .await
        .expect("trashed document");

    assert_eq!(granted.decision, Decision::Allowed);
    assert_eq!(ungranted.decision, Decision::NotFound);
    assert_eq!(trashed.decision, Decision::NotFound);
}
