//! Acta's resource provider inside the server's V2 authorization service
//! (`v2-e7-s1b-acta-provider`, ACTA-AUTHZ-1/5): through
//! `POST /api/v2/custos/authorize`, a V2 `editor@1` grant on a live document
//! allows while another document, or the same one in the trash, is not
//! found. Existence comes from Acta's own rows through the provider the
//! server registers, and the catalog is the registry's Acta declaration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::authorization::{AuthorizeDecision, AuthorizeRequest};
use atlas_client::AtlasClient;
use atlas_core::ids::ResourceRef;
use atlas_custos::entities::authorization::{
    GrantAuthority, GrantId, NewGrantRecord, SubjectRecord, TargetRecord,
};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::GrantV2Repo;
use atlas_custos_postgres::repos::authorization::PgGrantV2Repo;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

const DOCUMENT_UPDATE: &str = "acta::document::update";

fn document_ref(id: Uuid) -> ResourceRef {
    format!("acta::document::{id}")
        .parse()
        .expect("valid document ref")
}

/// A live document at the root of a fresh workspace.
async fn seed_document(db: &support::TestDb, name: &str) -> Uuid {
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

    document
}

async fn decide(client: &AtlasClient, action_raw: &str, document: Uuid) -> AuthorizeDecision {
    client
        .custos()
        .authorize(&AuthorizeRequest {
            action: action_raw.to_string(),
            target: document_ref(document).to_string(),
        })
        .await
        .expect("authorize answers")
        .decision
}

async fn trash_document(db: &support::TestDb, document: Uuid) {
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE acta.documents SET deleted_at = now() WHERE id = '{document}'"
        ))
        .await
        .expect("trash document");
}

#[tokio::test]
async fn an_editor_grant_on_an_acta_document_answers_through_the_authorize_route() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "acta-authorize-editor").await;
    let document = seed_document(&db, "acta-authorize-granted").await;
    let other = seed_document(&db, "acta-authorize-other").await;
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(PrincipalId::from(user.id)),
        target: TargetRecord::Ref(document_ref(document)),
        authority: GrantAuthority::Builtin {
            name: "editor".to_string(),
            version: 1,
        },
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed editor grant");

    let granted = decide(&client, DOCUMENT_UPDATE, document).await;
    let ungranted = decide(&client, DOCUMENT_UPDATE, other).await;
    trash_document(&db, document).await;
    let trashed = decide(&client, DOCUMENT_UPDATE, document).await;

    assert_eq!(granted, AuthorizeDecision::Allow);
    assert_eq!(ungranted, AuthorizeDecision::NotFound);
    assert_eq!(trashed, AuthorizeDecision::NotFound);
}
