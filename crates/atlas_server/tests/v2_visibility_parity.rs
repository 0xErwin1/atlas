//! V1/V2 list visibility parity harness (`v2-e7-s3-visibility-sql-parity`,
//! D-E7S3-2b, ACTA-MIG-5's authorization sample): one workspace is seeded
//! once as V1 memberships, project visibility and permission grants, and once
//! as the equivalent V2 rows. For every principal and every listing, what V1
//! serves through its HTTP routes (gate outcome, id set and page boundaries)
//! is compared with what the V2 list variant returns under the predicate the
//! authorization service compiles for that principal.
//!
//! The V1 -> V2 mapping (D-E7S3-2): an owner holds an explicit action set on
//! the workspace (admin@1's actions plus `workspace::transfer` and
//! `workspace::delete`, until S4 seeds the `acta:workspace-owner` role); an
//! admin holds `admin@1` on the workspace; a member is in the workspace
//! `members` set, which holds `<role>@1` on every `workspace(<role>)` project
//! and nothing on a private one; each V1 permission grant becomes a V2 grant of
//! the same role at version 1 on the same resource, for the same user, group
//! or the key's agent principal. Root sees everything in both models.
//!
//! Named divergences, where V2 deliberately differs from what V1 serves. The
//! ones the parity sweep meets (Q3, Q6, Q7, Q9) are rows of
//! [`named_divergences`], each with the exact ids V1 and V2 serve, and the
//! sweep asserts them; the others (Q2, Q4, Q5, Q8) sit outside the sweep's
//! listings and have one test each. E13's MIG-5 verification reuses this
//! list:
//! - Q2: the V1 document list SQL ignores project visibility and includes
//!   project-less documents; the V1 route only lists documents inside a
//!   project it gates, so a project-less document is unreachable through it,
//!   while V2 lists it where a workspace grant or a grant on it allows.
//! - Q3: the V1 list and search SQL ignore group grants that V1's resolver
//!   honors; V2 applies group grants when it evaluates.
//! - Q4: the V1 search SQL chains a task to its board, project and
//!   workspace, skipping folders; V2 paths place boards under their folder,
//!   so a folder grant surfaces the board's tasks in V2 search. (The V1
//!   board route's gate honors folder grants, so lists agree.)
//! - Q5: the V1 project list SQL shows non-private projects to any user, even
//!   a non-member; the V1 member gate masks it.
//! - Q6: the V1 workspace task list shows every top-level task to whoever
//!   passes its member gate (a plain member, an agent key with workspace
//!   access), private projects included; V2 closes that leak.
//! - Q7: a folder or board grant alone cannot pass V1's project or board
//!   gate, so V1 lists nothing, while V2 lists the rows the grant covers.
//! - Q8: V1 hides nothing below a gate; a V2 block rule or an enforced deny
//!   below it has no V1 counterpart (the enforced case runs separately,
//!   D-E7S3-5).
//! - Q9: every V1 list route requires workspace membership (or, for a key,
//!   workspace access) before its own gate, so a principal holding only a
//!   grant inside the workspace lists nothing through V1, while V2 lists the
//!   rows the grant covers.
//!
//! The members set is resolved by a harness provider reading
//! `acta.workspace_memberships`, the source Acta's own provider (E7-S1b)
//! answers from once it registers. An agent key's V2 ceiling is left
//! unrestricted: mapping its V1 scopes to V2 actions is E7-S2's; the key
//! carries every read scope so V1's scope check never refuses it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::ids::{BoardId, FolderId, ProjectId, WorkspaceId};
use atlas_acta::ports::documents::{DocumentRepo, FolderPresence};
use atlas_acta::ports::workspace_core::ProjectRepo;
use atlas_acta::search::{SearchQuery, SearchSort, TypeSet};
use atlas_acta_postgres::repos::documents::PgDocumentRepo;
use atlas_acta_postgres::repos::search::PgSearchRepo;
use atlas_acta_postgres::repos::visible_lists::{Page, PgVisibleListRepo};
use atlas_api::dtos::boards_tasks::WorkspaceTaskQueryParams;
use atlas_client::{AtlasClient, ClientError};
use atlas_core::capabilities::{CapabilityError, ProviderCatalog, ResourceFacts, ResourceProvider};
use atlas_core::ids::{ActionId, PrincipalSetId, ResourceRef};
use atlas_core::principal::Principal as V1Principal;
use atlas_core::visibility::ListVisibility;
use atlas_custos::WorkspaceScope;
use atlas_custos::authorize::{
    ActorContext, AuthorizationService, AuthorizationSettings, ProviderSet,
};
use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};
use atlas_custos::entities::authorization::{
    DenyRuleId, GrantAuthority, GrantId, NewDenyRecord, NewGrantRecord, SubjectRecord, TargetRecord,
};
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey};
use atlas_custos::eval::{Catalog, Ceiling, DenyMode};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::{DenyRuleRepo, GrantV2Repo};
use atlas_custos::ports::group_repo::GroupRepo;
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo};
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_custos_postgres::repos::permissions::PgGroupRepo;
use atlas_server::auth::tokens::{generate_api_key, hash_token};
use atlas_server::authz::v2_service::{TokioSleeper, validation_catalog};
use atlas_server::persistence::repos::PgProjectRepo;
use atlas_server::state::AppState;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};
use uuid::Uuid;

const LIMITS: [u32; 3] = [1, 2, 50];

type Service = AuthorizationService<PgAuthorizationFactsStore, PgGroupMembershipSource>;

// ---------------------------------------------------------------------------
// The members-set provider
// ---------------------------------------------------------------------------

/// Answers `acta::workspace::<id>::members` from `acta.workspace_memberships`,
/// every role, as canonical principal text. Nothing else is asked of it.
struct MembersProvider {
    conn: DatabaseConnection,
}

#[derive(Debug, FromQueryResult)]
struct MemberRow {
    user_id: Uuid,
}

#[async_trait]
impl ResourceProvider for MembersProvider {
    async fn validate_ref(&self, _resource: &ResourceRef) -> Result<bool, CapabilityError> {
        Err(CapabilityError::unavailable("parity harness provider"))
    }

    async fn path_of(&self, _resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        Err(CapabilityError::unavailable("parity harness provider"))
    }

    async fn ancestors(
        &self,
        _resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        Err(CapabilityError::unavailable("parity harness provider"))
    }

    async fn members_of(
        &self,
        set: &PrincipalSetId,
    ) -> Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError> {
        let workspace = Uuid::parse_str(set.scope().id())
            .map_err(|_| CapabilityError::not_found(set.to_string()))?;
        let rows = MemberRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT user_id FROM acta.workspace_memberships WHERE workspace_id = $1",
            [workspace.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(|error| CapabilityError::unavailable(error.to_string()))?;

        rows.into_iter()
            .map(|row| {
                atlas_core::ids::PrincipalId::new(&row.user_id.to_string())
                    .map_err(|error| CapabilityError::unavailable(error.to_string()))
            })
            .collect()
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        Err(CapabilityError::unavailable("parity harness provider"))
    }

    async fn resource_facts(
        &self,
        _resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        Err(CapabilityError::unavailable("parity harness provider"))
    }
}

// ---------------------------------------------------------------------------
// The seeded world
// ---------------------------------------------------------------------------

/// One project's rows: a root folder with a subfolder, a board at the
/// project root and one inside the root folder, their top-level tasks, and
/// documents at the project root and in the subfolder.
struct ProjectRows {
    id: Uuid,
    slug: String,
    root_folder: Uuid,
    subfolder: Uuid,
    boards: [Uuid; 2],
    /// Each board's top-level tasks, in id order.
    tasks: [Vec<Uuid>; 2],
    root_documents: Vec<Uuid>,
    subfolder_documents: Vec<Uuid>,
}

impl ProjectRows {
    fn all_tasks(&self) -> Vec<Uuid> {
        self.tasks.concat()
    }

    fn all_documents(&self) -> Vec<Uuid> {
        let mut ids = [
            self.root_documents.clone(),
            self.subfolder_documents.clone(),
        ]
        .concat();
        ids.sort();
        ids
    }
}

/// A principal with its V1 client and its V2 actor.
struct Actor2 {
    name: &'static str,
    client: AtlasClient,
    actor: ActorContext,
}

struct World {
    db: support::TestDb,
    _server: support::TestServer,
    state: AppState,
    workspace: Uuid,
    slug: String,
    owner: Uuid,
    public: ProjectRows,
    private: ProjectRows,
    workspace_document: Uuid,
    principals: Vec<Actor2>,
}

impl World {
    fn principal(&self, name: &str) -> &Actor2 {
        self.principals
            .iter()
            .find(|principal| principal.name == name)
            .unwrap_or_else(|| panic!("no principal {name}"))
    }

    fn ctx(&self) -> WorkspaceCtx {
        WorkspaceCtx::new(
            WorkspaceId(self.workspace),
            Actor::User(UserAttributionId(self.owner)),
        )
    }
}

async fn exec(db: &support::TestDb, sql: String) {
    db.conn()
        .execute_unprepared(&sql)
        .await
        .unwrap_or_else(|error| panic!("seed statement failed: {error}\n{sql}"));
}

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn user_actor(user: &atlas_server::persistence::repos::User) -> ActorContext {
    ActorContext {
        principal: PrincipalId::from(user.id),
        is_root: false,
        ceiling: Ceiling::Unrestricted,
    }
}

async fn member(db: &support::TestDb, workspace: Uuid, user: Uuid, role: &str) {
    exec(
        db,
        format!(
            "INSERT INTO acta.workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace}', '{user}', '{role}', now(), now())",
            Uuid::now_v7()
        ),
    )
    .await;
}

/// A V1 permission grant of `role` on `resource` for one subject column.
async fn v1_grant(
    db: &support::TestDb,
    workspace: Uuid,
    column: &str,
    subject: Uuid,
    resource: &str,
    role: &str,
) {
    exec(
        db,
        format!(
            "INSERT INTO custos.permission_grants (id, workspace_id, {column}, resource_ref, role, \
                 created_at, updated_at) \
             VALUES ('{}', '{workspace}', '{subject}', '{resource}', '{role}', now(), now())",
            Uuid::now_v7()
        ),
    )
    .await;
}

async fn v2_grant(
    db: &support::TestDb,
    subject: SubjectRecord,
    resource: &str,
    authority: GrantAuthority,
) {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject,
        target: TargetRecord::Ref(resource.parse().expect("valid target")),
        authority,
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed V2 grant");
}

fn builtin(name: &str) -> GrantAuthority {
    GrantAuthority::Builtin {
        name: name.to_string(),
        version: 1,
    }
}

/// Seeds one project with its folders, boards, tasks and documents. Rows
/// are created in id order and with increasing positions and creation
/// times, so every V1 ordering the routes use agrees with id order.
async fn seed_project(
    db: &support::TestDb,
    world_ctx: &WorkspaceCtx,
    owner: Uuid,
    slug: &str,
    prefix: &str,
    visibility: &str,
    recency_offset: usize,
) -> ProjectRows {
    let workspace = world_ctx.workspace_id.0;
    let project = Uuid::now_v7();
    let (root_folder, subfolder) = (Uuid::now_v7(), Uuid::now_v7());
    let boards = [Uuid::now_v7(), Uuid::now_v7()];
    let columns = [Uuid::now_v7(), Uuid::now_v7()];
    let role = if visibility == "workspace" {
        "'viewer'"
    } else {
        "NULL"
    };

    exec(
        db,
        format!(
            "INSERT INTO acta.projects (id, workspace_id, name, slug, task_prefix, next_task_number, \
                 visibility, visibility_role, created_by_user_id, created_at, updated_at) \
             VALUES ('{project}', '{workspace}', '{slug}', '{slug}', '{prefix}', 10, '{visibility}', {role}, \
                 '{owner}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, \
                 created_by_user_id, created_at, updated_at) \
             VALUES ('{root_folder}', '{workspace}', '{project}', NULL, 'Root', '{owner}', now(), now()), \
                    ('{subfolder}', '{workspace}', '{project}', '{root_folder}', 'Sub', '{owner}', now(), now()); \
             INSERT INTO acta.boards (id, workspace_id, project_id, folder_id, name, created_by_user_id, \
                 created_at, updated_at) \
             VALUES ('{b0}', '{workspace}', '{project}', NULL, 'Top', '{owner}', now(), now()), \
                    ('{b1}', '{workspace}', '{project}', '{root_folder}', 'Filed', '{owner}', now(), now()); \
             INSERT INTO acta.board_columns (id, workspace_id, board_id, name, position_key, \
                 created_by_user_id, created_at, updated_at) \
             VALUES ('{c0}', '{workspace}', '{b0}', 'Todo', 'a0', '{owner}', now(), now()), \
                    ('{c1}', '{workspace}', '{b1}', 'Todo', 'a0', '{owner}', now(), now())",
            b0 = boards[0],
            b1 = boards[1],
            c0 = columns[0],
            c1 = columns[1],
        ),
    )
    .await;

    let mut tasks: [Vec<Uuid>; 2] = [Vec::new(), Vec::new()];
    for (index, (board, column)) in boards.iter().zip(columns).enumerate() {
        tasks[index] = seed_tasks(
            db,
            workspace,
            owner,
            project,
            *board,
            column,
            prefix,
            index,
            recency_offset,
        )
        .await;
    }

    let mut documents = [Vec::new(), Vec::new()];
    for (slot, folder) in [None, Some(subfolder)].into_iter().enumerate() {
        for _ in 0..2 {
            documents[slot].push(create_document(db, world_ctx, Some(project), folder).await);
        }
    }
    let [root_documents, subfolder_documents] = documents;

    ProjectRows {
        id: project,
        slug: slug.to_string(),
        root_folder,
        subfolder,
        boards,
        tasks,
        root_documents,
        subfolder_documents,
    }
}

/// Three top-level tasks and one subtask on `board`; returns the top-level
/// ones in id order. Each later task is
/// less recently updated, so the V1 workspace list's default order (most
/// recently updated first) is id order, the order V2 pages in.
#[allow(clippy::too_many_arguments)]
async fn seed_tasks(
    db: &support::TestDb,
    workspace: Uuid,
    owner: Uuid,
    project: Uuid,
    board: Uuid,
    column: Uuid,
    prefix: &str,
    board_index: usize,
    offset: usize,
) -> Vec<Uuid> {
    let mut parent = None;
    let mut top_level = Vec::new();

    for n in 0..4 {
        let task = Uuid::now_v7();
        let number = board_index * 10 + n + 1;
        let recency = offset + number;
        let parent_value = match (n, parent) {
            (3, Some(parent)) => format!("'{parent}'"),
            _ => "NULL".to_string(),
        };
        exec(
            db,
            format!(
                "INSERT INTO acta.tasks (id, workspace_id, project_id, board_id, column_id, parent_task_id, \
                     readable_id, title, position_key, created_by_user_id, created_at, updated_at) \
                 VALUES ('{task}', '{workspace}', '{project}', '{board}', '{column}', {parent_value}, \
                     '{prefix}-{number}', 'Task', 'a{number:03}', '{owner}', \
                     now(), now() - interval '{recency} seconds')"
            ),
        )
        .await;
        parent.get_or_insert(task);
        if n < 3 {
            top_level.push(task);
        }
    }

    top_level
}

async fn create_document(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    project: Option<Uuid>,
    folder: Option<Uuid>,
) -> Uuid {
    PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            ctx,
            NewDocument {
                title: "Doc".to_string(),
                slug: None,
                content: String::new(),
                folder_id: folder.map(FolderId),
                project_id: project.map(ProjectId),
                frontmatter: None,
            },
        )
        .await
        .expect("seed document")
        .id
        .0
}

#[derive(Debug, FromQueryResult)]
struct KeyPrincipal {
    principal_id: Uuid,
}

/// An agent key in the workspace carrying every read scope, and the agent
/// principal it acts as.
async fn agent_key(
    server: &support::TestServer,
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
) -> (AtlasClient, Uuid, PrincipalId) {
    let raw_token = generate_api_key();
    let scopes = [
        CapabilityFamily::Projects,
        CapabilityFamily::Docs,
        CapabilityFamily::Folders,
        CapabilityFamily::Boards,
        CapabilityFamily::Tasks,
    ]
    .into_iter()
    .map(|family| Capability {
        family,
        action: CapabilityAction::Read,
    })
    .collect();

    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "parity-agent".to_string(),
            token_hash: hash_token(&raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes,
        },
    )
    .await
    .expect("create api key");

    let principal = KeyPrincipal::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT principal_id FROM custos.api_keys WHERE id = '{}'",
            key.id.0
        ),
    ))
    .one(db.conn())
    .await
    .expect("query key principal")
    .expect("key row")
    .principal_id;

    (
        AtlasClient::new(server.base_url().to_string()).with_token(raw_token),
        key.id.0,
        PrincipalId(principal),
    )
}

/// The owner's explicit action set: admin@1's actions plus the owner-only
/// workspace actions.
fn owner_authority(catalog: &Catalog) -> GrantAuthority {
    let mut actions: Vec<ActionId> = catalog
        .builtin_role("acta", "admin", 1)
        .expect("acta declares admin@1")
        .actions()
        .iter()
        .cloned()
        .collect();
    actions.push(action("acta::workspace::transfer"));
    actions.push(action("acta::workspace::delete"));

    GrantAuthority::Actions(actions)
}

async fn seed_world() -> World {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let catalog = validation_catalog(&state.registry).expect("catalog");

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "parity-owner").await;
    let workspace = ws.id.0;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(UserAttributionId(owner.id.0)));
    let public = seed_project(&db, &ctx, owner.id.0, "public", "PU", "workspace", 0).await;
    let private = seed_project(&db, &ctx, owner.id.0, "private", "PR", "private", 100).await;
    let workspace_document = create_document(&db, &ctx, None, None).await;

    let (admin_client, admin) = support::login_user(&server, &db, "parity-admin").await;
    let (member_client, plain) = support::login_user(&server, &db, "parity-member").await;
    let (outsider_client, outsider) = support::login_user(&server, &db, "parity-outsider").await;
    let (grouped_client, grouped) = support::login_user(&server, &db, "parity-grouped").await;
    let (stranger_client, stranger) = support::login_user(&server, &db, "parity-stranger").await;
    let (foldered_client, foldered) = support::login_user(&server, &db, "parity-foldered").await;
    let root_client = support::login_root_user(&server, &db).await;
    let (key_client, key_id, key_principal) = agent_key(&server, &db, &ctx).await;

    member(&db, workspace, admin.id.0, "admin").await;
    member(&db, workspace, plain.id.0, "member").await;
    member(&db, workspace, grouped.id.0, "member").await;
    member(&db, workspace, foldered.id.0, "member").await;

    let group = PgGroupRepo {
        conn: db.conn().clone(),
    };
    let group_id = group
        .create(NewGroup {
            workspace_id: WorkspaceScope(workspace),
            name: "parity".to_string(),
            created_by: owner.id,
        })
        .await
        .expect("seed group")
        .id;
    exec(
        &db,
        format!(
            "INSERT INTO custos.group_members (group_id, user_id, created_at) \
             VALUES ('{}', '{}', now())",
            group_id.0, grouped.id.0
        ),
    )
    .await;

    let ws_ref = format!("acta::workspace::{workspace}");
    let public_ref = format!("acta::project::{}", public.id);
    let private_ref = format!("acta::project::{}", private.id);

    v1_grant(
        &db,
        workspace,
        "user_id",
        outsider.id.0,
        &private_ref,
        "viewer",
    )
    .await;
    v1_grant(
        &db,
        workspace,
        "group_id",
        group_id.0,
        &private_ref,
        "viewer",
    )
    .await;
    v1_grant(&db, workspace, "api_key_id", key_id, &public_ref, "viewer").await;
    let private_folder_ref = format!("acta::folder::{}", private.root_folder);
    v1_grant(
        &db,
        workspace,
        "user_id",
        foldered.id.0,
        &private_folder_ref,
        "viewer",
    )
    .await;

    let members: PrincipalSetId = format!("{ws_ref}::members").parse().unwrap();
    v2_grant(
        &db,
        SubjectRecord::Principal(PrincipalId::from(owner.id)),
        &ws_ref,
        owner_authority(&catalog),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::Principal(PrincipalId::from(admin.id)),
        &ws_ref,
        builtin("admin"),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::PrincipalSet(members),
        &public_ref,
        builtin("viewer"),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::Principal(PrincipalId::from(outsider.id)),
        &private_ref,
        builtin("viewer"),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::Group(group_id),
        &private_ref,
        builtin("viewer"),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::Principal(PrincipalId::from(foldered.id)),
        &private_folder_ref,
        builtin("viewer"),
    )
    .await;
    v2_grant(
        &db,
        SubjectRecord::Principal(key_principal),
        &public_ref,
        builtin("viewer"),
    )
    .await;

    let principals = vec![
        Actor2 {
            name: "owner",
            client: owner_client,
            actor: user_actor(&owner),
        },
        Actor2 {
            name: "admin",
            client: admin_client,
            actor: user_actor(&admin),
        },
        Actor2 {
            name: "member",
            client: member_client,
            actor: user_actor(&plain),
        },
        Actor2 {
            name: "outsider",
            client: outsider_client,
            actor: user_actor(&outsider),
        },
        Actor2 {
            name: "grouped",
            client: grouped_client,
            actor: user_actor(&grouped),
        },
        Actor2 {
            name: "foldered",
            client: foldered_client,
            actor: user_actor(&foldered),
        },
        Actor2 {
            name: "stranger",
            client: stranger_client,
            actor: user_actor(&stranger),
        },
        Actor2 {
            name: "root",
            client: root_client,
            actor: ActorContext {
                principal: PrincipalId::new(),
                is_root: true,
                ceiling: Ceiling::Unrestricted,
            },
        },
        Actor2 {
            name: "agent",
            client: key_client,
            actor: ActorContext {
                principal: key_principal,
                is_root: false,
                ceiling: Ceiling::Unrestricted,
            },
        },
    ];

    World {
        db,
        _server: server,
        state,
        workspace,
        slug: ws.slug,
        owner: owner.id.0,
        public,
        private,
        workspace_document,
        principals,
    }
}

fn service(world: &World, deny_mode: DenyMode) -> Service {
    let conn = world.db.conn().clone();

    AuthorizationService::new(
        ProviderSet::new().with("acta", Arc::new(MembersProvider { conn: conn.clone() })),
        PgAuthorizationFactsStore { conn: conn.clone() },
        PgGroupMembershipSource { conn },
        Arc::new(TokioSleeper),
        AuthorizationSettings {
            catalog: validation_catalog(&world.state.registry).expect("catalog"),
            deny_mode,
            provider_timeout: Duration::from_secs(5),
        },
    )
}

// ---------------------------------------------------------------------------
// Listings, as V1 serves them and as V2 selects them
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    Projects,
    Documents(Uuid),
    Folders(Uuid),
    Boards(Uuid),
    BoardTasks(Uuid),
    WorkspaceTasks,
}

impl Listing {
    fn kind(self) -> &'static str {
        match self {
            Self::Projects => "project",
            Self::Documents(_) => "document",
            Self::Folders(_) => "folder",
            Self::Boards(_) => "board",
            Self::BoardTasks(_) | Self::WorkspaceTasks => "task",
        }
    }
}

/// Every listing of the world: the projects, each project's documents,
/// folders and boards, each board's tasks, and the workspace task list.
fn listings(world: &World) -> Vec<Listing> {
    let mut listings = vec![Listing::Projects, Listing::WorkspaceTasks];

    for project in [&world.public, &world.private] {
        listings.push(Listing::Documents(project.id));
        listings.push(Listing::Folders(project.id));
        listings.push(Listing::Boards(project.id));
        listings.extend(project.boards.map(Listing::BoardTasks));
    }

    listings
}

/// The pages a listing serves: a refused gate serves no page at all.
type Pages = Vec<Vec<Uuid>>;

fn slug_of(world: &World, project: Uuid) -> &str {
    if project == world.public.id {
        &world.public.slug
    } else {
        &world.private.slug
    }
}

fn refusal(error: ClientError, what: &str) -> Pages {
    match error {
        ClientError::Api(problem) if matches!(problem.status, 403 | 404) => Vec::new(),
        other => panic!("{what}: unexpected V1 failure: {other}"),
    }
}

/// One V1 page: its ids and the cursor to the next one, or the gate's
/// refusal.
async fn v1_page(
    world: &World,
    client: &AtlasClient,
    listing: Listing,
    cursor: Option<&str>,
    limit: u32,
) -> Result<(Vec<Uuid>, Option<String>), ClientError> {
    let ws = world.slug.as_str();

    macro_rules! page {
        ($call:expr) => {{
            let page = $call.await?;
            Ok((
                page.items.iter().map(|item| item.id).collect(),
                page.next_cursor,
            ))
        }};
    }

    match listing {
        Listing::Projects => page!(client.acta().list_projects(ws, cursor, Some(limit))),
        Listing::Documents(project) => {
            page!(
                client
                    .acta()
                    .list_documents(ws, slug_of(world, project), cursor, Some(limit))
            )
        }
        Listing::Folders(project) => {
            page!(
                client
                    .acta()
                    .list_folders(ws, slug_of(world, project), cursor, Some(limit))
            )
        }
        Listing::Boards(project) => {
            page!(
                client
                    .acta()
                    .list_boards(ws, slug_of(world, project), cursor, Some(limit))
            )
        }
        Listing::BoardTasks(board) => {
            page!(client.acta().list_tasks(ws, board, cursor, Some(limit)))
        }
        Listing::WorkspaceTasks => page!(client.acta().list_workspace_tasks(
            ws,
            &WorkspaceTaskQueryParams {
                cursor: cursor.map(ToString::to_string),
                limit: Some(limit),
                ..WorkspaceTaskQueryParams::default()
            }
        )),
    }
}

async fn v1_served(world: &World, principal: &Actor2, listing: Listing, limit: u32) -> Pages {
    let mut pages: Pages = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        match v1_page(world, &principal.client, listing, cursor.as_deref(), limit).await {
            Ok((ids, next)) => {
                pages.push(ids);
                match next {
                    Some(next) => cursor = Some(next),
                    None => return pages,
                }
            }
            Err(error) => return refusal(error, &format!("{} {listing:?}", principal.name)),
        }
    }
}

/// The ids one V2 page selects, fetched with one extra row as the V1 routes
/// do to learn whether another page follows.
async fn v2_rows(
    world: &World,
    visibility: &ListVisibility,
    listing: Listing,
    page: Page,
) -> Vec<Uuid> {
    let lists = PgVisibleListRepo {
        conn: world.db.conn().clone(),
    };
    let ctx = world.ctx();

    match listing {
        Listing::Projects => lists
            .list_projects_v2(&ctx, visibility, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        Listing::Documents(project) => PgDocumentRepo::new(world.db.conn().clone(), 50)
            .list_visible_v2_with_folder_presence(
                &ctx,
                visibility,
                Some(ProjectId(project)),
                FolderPresence::Any,
                page.after_id,
                page.limit,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        Listing::Folders(project) => lists
            .list_folders_v2(&ctx, visibility, Some(ProjectId(project)), page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        Listing::Boards(project) => lists
            .list_boards_v2(&ctx, visibility, Some(ProjectId(project)), page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        Listing::BoardTasks(board) => lists
            .list_tasks_v2(&ctx, visibility, Some(BoardId(board)), page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        Listing::WorkspaceTasks => lists
            .list_tasks_v2(&ctx, visibility, None, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
    }
}

async fn visibility_for(service: &Service, principal: &Actor2, listing: Listing) -> ListVisibility {
    let kind = listing.kind();
    let predicate = service
        .visibility_filter(
            &principal.actor,
            kind,
            &action(&format!("acta::{kind}::read")),
        )
        .await
        .unwrap_or_else(|error| panic!("{} {listing:?}: visibility: {error:?}", principal.name));

    ListVisibility::from(&predicate)
}

async fn v2_served(
    world: &World,
    service: &Service,
    principal: &Actor2,
    listing: Listing,
    limit: u32,
) -> Pages {
    let visibility = visibility_for(service, principal, listing).await;
    let mut pages: Pages = Vec::new();
    let mut after_id = None;

    loop {
        let page = Page {
            after_id,
            limit: u64::from(limit) + 1,
        };
        let mut ids = v2_rows(world, &visibility, listing, page).await;
        let has_more = ids.len() > limit as usize;
        ids.truncate(limit as usize);

        after_id = ids.last().copied();
        pages.push(ids);

        if !has_more {
            return pages;
        }
    }
}

/// Normalizes "refused" and "one empty page" to the same served nothing.
fn normalized(pages: Pages) -> Pages {
    if pages.iter().all(Vec::is_empty) {
        Vec::new()
    } else {
        pages
    }
}

// ---------------------------------------------------------------------------
// Parity
// ---------------------------------------------------------------------------

/// One (principal, listing) pair where V2 deliberately serves something
/// else than V1, with the exact ids each serves, in serving order. E13's
/// MIG-5 verification reuses this table.
struct Divergence {
    principal: &'static str,
    listing: Listing,
    code: &'static str,
    v1: Vec<Uuid>,
    v2: Vec<Uuid>,
}

fn divergence(
    principal: &'static str,
    listing: Listing,
    code: &'static str,
    v1: Vec<Uuid>,
    v2: Vec<Uuid>,
) -> Divergence {
    Divergence {
        principal,
        listing,
        code,
        v1,
        v2,
    }
}

/// The named divergences the sweep meets on the seeded world. Every other
/// (principal, listing) pair must serve the same pages in both models.
fn named_divergences(world: &World) -> Vec<Divergence> {
    let (public, private) = (&world.public, &world.private);
    let every_task = [public.all_tasks(), private.all_tasks()].concat();
    let private_folders = vec![private.root_folder, private.subfolder];

    let mut table = vec![
        divergence(
            "grouped",
            Listing::Projects,
            "Q3",
            vec![public.id],
            vec![public.id, private.id],
        ),
        divergence(
            "member",
            Listing::WorkspaceTasks,
            "Q6",
            every_task.clone(),
            public.all_tasks(),
        ),
        divergence(
            "agent",
            Listing::WorkspaceTasks,
            "Q6",
            every_task.clone(),
            public.all_tasks(),
        ),
        divergence(
            "foldered",
            Listing::WorkspaceTasks,
            "Q6",
            every_task,
            [public.all_tasks(), private.tasks[1].clone()].concat(),
        ),
        divergence(
            "foldered",
            Listing::Documents(private.id),
            "Q7",
            Vec::new(),
            private.subfolder_documents.clone(),
        ),
        divergence(
            "foldered",
            Listing::Folders(private.id),
            "Q7",
            Vec::new(),
            private_folders.clone(),
        ),
        divergence(
            "foldered",
            Listing::Boards(private.id),
            "Q7",
            Vec::new(),
            vec![private.boards[1]],
        ),
        divergence(
            "outsider",
            Listing::Projects,
            "Q9",
            Vec::new(),
            vec![private.id],
        ),
        divergence(
            "outsider",
            Listing::WorkspaceTasks,
            "Q9",
            Vec::new(),
            private.all_tasks(),
        ),
        divergence(
            "outsider",
            Listing::Documents(private.id),
            "Q9",
            Vec::new(),
            private.all_documents(),
        ),
        divergence(
            "outsider",
            Listing::Folders(private.id),
            "Q9",
            Vec::new(),
            private_folders,
        ),
        divergence(
            "outsider",
            Listing::Boards(private.id),
            "Q9",
            Vec::new(),
            private.boards.to_vec(),
        ),
    ];

    for (board, tasks) in private.boards.iter().zip(&private.tasks) {
        table.push(divergence(
            "outsider",
            Listing::BoardTasks(*board),
            "Q9",
            Vec::new(),
            tasks.clone(),
        ));
    }

    table
}

/// `ids` served `limit` at a time; nothing at all when there is no id.
fn chunked(ids: &[Uuid], limit: u32) -> Pages {
    ids.chunks(limit as usize).map(<[Uuid]>::to_vec).collect()
}

/// The pages each model is expected to serve for `listing`: the table's ids
/// for a named divergence, otherwise V1's own pages for both.
fn expected_pages(
    divergences: &[Divergence],
    principal: &str,
    listing: Listing,
    limit: u32,
    v1: &Pages,
) -> (Pages, Pages, &'static str) {
    match divergences
        .iter()
        .find(|row| row.principal == principal && row.listing == listing)
    {
        Some(row) => (chunked(&row.v1, limit), chunked(&row.v2, limit), row.code),
        None => (v1.clone(), v1.clone(), "parity"),
    }
}

#[tokio::test]
async fn v2_serves_what_v1_serves_except_the_named_divergences() {
    let world = seed_world().await;
    let service = service(&world, DenyMode::Disabled);
    let divergences = named_divergences(&world);
    let mut mismatches: Vec<String> = Vec::new();
    let mut met: Vec<(&str, Listing)> = Vec::new();
    let mut compared = 0usize;

    for principal in &world.principals {
        for listing in listings(&world) {
            for limit in LIMITS {
                let v1 = normalized(v1_served(&world, principal, listing, limit).await);
                let v2 = normalized(v2_served(&world, &service, principal, listing, limit).await);
                let (expected_v1, expected_v2, code) =
                    expected_pages(&divergences, principal.name, listing, limit, &v1);
                compared += 1;

                if code != "parity" {
                    met.push((principal.name, listing));
                }
                if v1 != expected_v1 || v2 != expected_v2 {
                    mismatches.push(format!(
                        "{} {listing:?} limit {limit} ({code}): V1 {v1:?} expected {expected_v1:?}; \
                         V2 {v2:?} expected {expected_v2:?}",
                        principal.name
                    ));
                }
            }
        }
    }

    let unmet: Vec<String> = divergences
        .iter()
        .filter(|row| !met.contains(&(row.principal, row.listing)))
        .map(|row| format!("{} {:?} ({})", row.principal, row.listing, row.code))
        .collect();

    assert!(compared > 100, "only {compared} comparisons");
    assert!(unmet.is_empty(), "named divergences never met: {unmet:?}");
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

// ---------------------------------------------------------------------------
// Named divergences
// ---------------------------------------------------------------------------

/// Every id V1 and V2 serve for `listing` to `principal`, at one page size
/// large enough for the whole world.
async fn both_ids(
    world: &World,
    service: &Service,
    principal: &str,
    listing: Listing,
) -> (Vec<Uuid>, Vec<Uuid>) {
    let principal = world.principal(principal);
    let v1 = v1_served(world, principal, listing, 50).await.concat();
    let v2 = v2_served(world, service, principal, listing, 50)
        .await
        .concat();

    (v1, v2)
}

/// Q2: the V1 document route only lists inside a gated project, so a
/// project-less document never reaches a V1 list; V2 lists it for the owner,
/// whose workspace grant covers it, and not for a plain member.
#[tokio::test]
async fn q2_project_less_documents_are_unreachable_through_v1_and_listed_by_v2_grants() {
    let world = seed_world().await;
    let service = service(&world, DenyMode::Disabled);
    let owner = world.principal("owner");
    let mut v1_documents = Vec::new();
    for project in [world.public.id, world.private.id] {
        v1_documents.extend(
            v1_served(&world, owner, Listing::Documents(project), 50)
                .await
                .concat(),
        );
    }

    let unfiltered = |name: &'static str| {
        let world = &world;
        let service = &service;
        async move {
            let visibility = visibility_for(
                service,
                world.principal(name),
                Listing::Documents(Uuid::nil()),
            )
            .await;
            PgDocumentRepo::new(world.db.conn().clone(), 50)
                .list_visible_v2_with_folder_presence(
                    &world.ctx(),
                    &visibility,
                    None,
                    FolderPresence::Any,
                    None,
                    50,
                )
                .await
                .unwrap()
                .into_iter()
                .map(|document| document.id.0)
                .collect::<Vec<_>>()
        }
    };

    assert!(!v1_documents.contains(&world.workspace_document));
    assert!(
        unfiltered("owner")
            .await
            .contains(&world.workspace_document)
    );
    assert!(
        !unfiltered("member")
            .await
            .contains(&world.workspace_document)
    );
}

/// Q4: V1's search SQL chains a task to its board, project and workspace
/// and skips folders, so a grant on the folder holding a board does not
/// surface the board's tasks in V1 search; V2 places the board under the
/// folder. The V1 board route itself honors the folder grant, so the list
/// parity sweep covers that route.
#[tokio::test]
async fn folder_grants_reach_boards_and_tasks_under_v2() {
    let world = seed_world().await;
    let service = service(&world, DenyMode::Disabled);
    let foldered = world.principal("foldered");
    let filed_tasks = world.private.tasks[1].clone();

    let v1: Vec<Uuid> = foldered
        .client
        .acta()
        .search(
            &world.slug,
            "Task",
            Some("tasks"),
            None,
            None,
            Some(200),
            None,
        )
        .await
        .expect("V1 search")
        .items
        .into_iter()
        .map(|hit| hit.id)
        .collect();
    let tasks = visibility_for(&service, foldered, Listing::WorkspaceTasks).await;
    let v2: Vec<Uuid> = PgSearchRepo::new(world.db.conn().clone())
        .search_v2(
            &world.ctx(),
            &SearchQuery {
                text: "Task".to_string(),
                filters: Vec::new(),
                sort: SearchSort::Relevance,
                type_filter: TypeSet {
                    notes: false,
                    tasks: true,
                },
                warnings: Vec::new(),
                prefix: false,
            },
            200,
            None,
            &ListVisibility::Nothing,
            &tasks,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|hit| hit.id)
        .collect();

    assert_eq!(filed_tasks.len(), 3);
    assert!(filed_tasks.iter().all(|task| !v1.contains(task)));
    assert!(filed_tasks.iter().all(|task| v2.contains(task)));
}

/// Q5: V1's project list SQL shows the non-private project to a user who is
/// not a member; the V1 member gate masks it, and V2 shows nothing.
#[tokio::test]
async fn q5_the_member_gate_masks_the_v1_project_sql_for_non_members() {
    let world = seed_world().await;
    let service = service(&world, DenyMode::Disabled);
    let stranger = world.principal("stranger");
    let stranger_user = atlas_core::principal::UserId(stranger.actor.principal.0);

    let repo_rows = PgProjectRepo {
        conn: world.db.conn().clone(),
    }
    .list_visible(&world.ctx(), &V1Principal::User(stranger_user), None, 50)
    .await
    .unwrap();
    let (v1, v2) = both_ids(&world, &service, "stranger", Listing::Projects).await;

    assert_eq!(
        repo_rows
            .iter()
            .map(|project| project.id.0)
            .collect::<Vec<_>>(),
        vec![world.public.id]
    );
    assert!(v1.is_empty());
    assert!(v2.is_empty());
}

/// Q8: V1 hides nothing below its gate. A V2 grant on a folder without the
/// read action wins the folder's nearest level and hides it and its
/// subfolder from the member, while V1 still lists both.
#[tokio::test]
async fn q8_v1_hides_nothing_below_its_gate() {
    let world = seed_world().await;
    let member = world.principal("member").actor.principal;
    v2_grant(
        &world.db,
        SubjectRecord::Principal(member),
        &format!("acta::folder::{}", world.public.root_folder),
        GrantAuthority::Actions(vec![action("acta::folder::update")]),
    )
    .await;
    let service = service(&world, DenyMode::Disabled);

    let (v1, v2) = both_ids(
        &world,
        &service,
        "member",
        Listing::Folders(world.public.id),
    )
    .await;

    assert_eq!(v1, vec![world.public.root_folder, world.public.subfolder]);
    assert!(v2.is_empty());
}

/// D-E7S3-5: under enforced denies, a deny on a folder removes the folder,
/// its subfolder and the documents below it from the owner's V2 lists, as
/// `permits` decides; V1 has no denies and still lists them.
#[tokio::test]
async fn an_enforced_ancestor_deny_excludes_the_subtree_in_sql() {
    let world = seed_world().await;
    let owner = world.principal("owner");
    PgDenyRuleRepo {
        conn: world.db.conn().clone(),
    }
    .create(NewDenyRecord {
        id: DenyRuleId::new(),
        subject: SubjectRecord::Principal(owner.actor.principal),
        target: TargetRecord::Ref(
            format!("acta::folder::{}", world.public.root_folder)
                .parse()
                .unwrap(),
        ),
        actions: vec![action("acta::folder::read"), action("acta::document::read")],
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed deny");
    let enforced = service(&world, DenyMode::Enforced);
    let public = world.public.id;

    let (v1_folders, v2_folders) =
        both_ids(&world, &enforced, "owner", Listing::Folders(public)).await;
    let (v1_documents, v2_documents) =
        both_ids(&world, &enforced, "owner", Listing::Documents(public)).await;
    let (_, v2_boards) = both_ids(&world, &enforced, "owner", Listing::Boards(public)).await;

    assert_eq!(v1_folders.len(), 2);
    assert!(v2_folders.is_empty());
    assert_eq!(v1_documents.len(), 4);
    assert_eq!(v2_documents.len(), 2, "only the project-root documents");
    assert_eq!(v2_boards.len(), 2, "board::read is not denied");
}
