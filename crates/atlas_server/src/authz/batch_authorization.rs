use std::collections::BTreeSet;

use crate::authz::ResourceRole;
use crate::authz::policy::{ChainSegment, ResolutionInput, ResourceChain, resolve};
use async_trait::async_trait;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::Visibility;
use atlas_acta::permissions::VisibilityRole;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::Principal;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::capability::CapabilityAction;
use atlas_custos::capability::CapabilityFamily;
use atlas_postgres::db_err as db_error;
use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};
use serde::Deserialize;
use uuid::Uuid;

/// Request-bound authorization state for viewer-relative projections.
///
/// This value is constructed only by `Authorized` after middleware has validated
/// the request principal. Its fields deliberately remain private so handlers
/// cannot turn caller-provided identifiers into authorization state.
#[derive(Clone)]
pub(crate) struct ProjectionAuthContext {
    workspace_id: WorkspaceId,
    principal: Principal,
}

impl ProjectionAuthContext {
    pub(super) fn from_validated(workspace_id: WorkspaceId, principal: Principal) -> Self {
        Self {
            workspace_id,
            principal,
        }
    }

    pub(crate) fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProjectionSubject {
    Document(Uuid),
    Task(Uuid),
    Attachment(Uuid),
    SourceComment(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectFamily {
    Documents,
    Tasks,
}

pub(crate) struct SubjectFact {
    pub ordinal: usize,
    pub chain: ResourceChain,
    pub family: SubjectFamily,
}

#[derive(Debug, Clone)]
pub(crate) struct UserFacts {
    pub user_id: UserId,
    pub is_active: bool,
    pub effective_membership: Option<MemberRole>,
    pub is_root_admin: bool,
    pub grants: Vec<(ResourceRef, ResourceRole)>,
}

#[derive(Debug, Clone)]
pub(crate) struct ApiKeyFacts {
    pub key_id: ApiKeyId,
    pub is_active: bool,
    pub is_revoked: bool,
    pub is_expired: bool,
    pub is_global: bool,
    pub scopes: Vec<Capability>,
    pub grants: Vec<(ResourceRef, ResourceRole)>,
    pub creator: UserFacts,
}

#[derive(Debug, Clone)]
pub(crate) enum PrincipalFacts {
    User(UserFacts),
    ApiKey(ApiKeyFacts),
}

/// Acta-implemented half of the batch authorization seam: resolves the
/// resource chain and visibility for a batch of subjects (`QUERY_A`).
///
/// Declared server-internal per design D2 — `atlas_core::ResourceProvider`
/// cannot carry this workload (no batching, no visibility, not an ancestors
/// query), and this seam is a V1-specific shape that does not belong on the
/// E3/E4 platform contract.
#[async_trait]
pub(crate) trait ResourceChainSource: Send + Sync {
    async fn load_subject_facts(
        &self,
        workspace: WorkspaceId,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError>;
}

/// Custos-implemented half of the batch authorization seam: resolves
/// principal facts and grants for a resolved resource set (`QUERY_B`).
///
/// `resources` crosses this boundary in the canonical `atlas_core` encoding;
/// the Acta-side codec (`atlas_acta::permissions::resource_ref_codec`)
/// converts at the edge so this port never speaks the V1 `ResourceRef` enum.
#[async_trait]
pub(crate) trait PrincipalFactsSource: Send + Sync {
    async fn load_principal_facts(
        &self,
        workspace: atlas_custos::WorkspaceScope,
        principal: &Principal,
        resources: &[atlas_core::ids::ResourceRef],
    ) -> Result<PrincipalFacts, DomainError>;
}

/// Postgres implementation of [`ResourceChainSource`], composed into
/// [`PgBatchAuthorizationSource`].
pub(crate) struct PgResourceChainSource {
    conn: DatabaseConnection,
}

/// Postgres implementation of [`PrincipalFactsSource`], composed into
/// [`PgBatchAuthorizationSource`].
pub(crate) struct PgPrincipalFactsSource {
    conn: DatabaseConnection,
}

/// Composition root for the batch authorization seam: holds one Acta-side
/// [`ResourceChainSource`] and one Custos-side [`PrincipalFactsSource`], and
/// exposes them together as a [`BatchAuthorizationSource`] so
/// `BatchAuthorizationService` and its callers keep a single source handle.
pub(crate) struct PgBatchAuthorizationSource {
    chain: PgResourceChainSource,
    facts: PgPrincipalFactsSource,
}

impl PgBatchAuthorizationSource {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self {
            chain: PgResourceChainSource { conn: conn.clone() },
            facts: PgPrincipalFactsSource { conn },
        }
    }
}

impl PrincipalFacts {
    #[cfg(test)]
    pub(super) fn active_user(
        membership: Option<MemberRole>,
        grants: Vec<(ResourceRef, ResourceRole)>,
    ) -> Self {
        Self::User(UserFacts {
            user_id: UserId(Uuid::nil()),
            is_active: true,
            effective_membership: membership,
            is_root_admin: false,
            grants,
        })
    }
}

#[async_trait]
pub(crate) trait BatchAuthorizationSource: Send + Sync {
    async fn load_subject_facts(
        &self,
        context: &ProjectionAuthContext,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError>;

    async fn load_principal_facts(
        &self,
        context: &ProjectionAuthContext,
        resources: &[ResourceRef],
    ) -> Result<PrincipalFacts, DomainError>;
}

#[derive(FromQueryResult)]
struct PrincipalFactsRow {
    facts: serde_json::Value,
}

#[derive(FromQueryResult)]
struct SubjectFactsRow {
    fact: serde_json::Value,
}

#[derive(Deserialize)]
struct StoredSubjectFact {
    ordinal: usize,
    family: String,
    chain: Vec<StoredChainSegment>,
}

#[derive(Deserialize)]
struct StoredChainSegment {
    resource: String,
    visibility: Option<StoredVisibility>,
}

#[derive(Deserialize)]
struct StoredVisibility {
    visibility: String,
    role: Option<String>,
}

#[derive(Deserialize)]
struct StoredGrant {
    resource: String,
    role: String,
}

#[derive(Deserialize)]
struct StoredUserFacts {
    user_id: Uuid,
    is_active: bool,
    effective_membership: Option<String>,
    is_root_admin: bool,
    grants: Vec<StoredGrant>,
}

#[derive(Deserialize)]
struct StoredApiKeyFacts {
    key_id: Uuid,
    is_active: bool,
    is_revoked: bool,
    is_expired: bool,
    is_global: bool,
    scopes: Vec<String>,
    grants: Vec<StoredGrant>,
    creator: StoredUserFacts,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredPrincipalFacts {
    User(StoredUserFacts),
    ApiKey(StoredApiKeyFacts),
}

#[async_trait]
impl ResourceChainSource for PgResourceChainSource {
    async fn load_subject_facts(
        &self,
        workspace: WorkspaceId,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError> {
        let rows = SubjectFactsRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            QUERY_A_SUBJECT_FACTS,
            [workspace.0.into(), subjects_json(subjects).into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_error)?;

        rows.into_iter()
            .map(|row| {
                serde_json::from_value(row.fact)
                    .map_err(|error| DomainError::Internal {
                        message: format!("invalid batch subject facts payload: {error}"),
                    })
                    .and_then(|fact| decode_subject_fact(fact, workspace))
            })
            .collect()
    }
}

#[async_trait]
impl PrincipalFactsSource for PgPrincipalFactsSource {
    async fn load_principal_facts(
        &self,
        workspace: atlas_custos::WorkspaceScope,
        principal: &Principal,
        resources: &[atlas_core::ids::ResourceRef],
    ) -> Result<PrincipalFacts, DomainError> {
        let workspace = WorkspaceId(workspace.0);

        let (principal_type, principal_id) = match principal {
            Principal::User(user_id) => ("user", user_id.0),
            Principal::ApiKey(key_id) => ("api_key", key_id.0),
            Principal::Group(_) => {
                return Err(DomainError::Internal {
                    message: "groups cannot authorize comment projections".into(),
                });
            }
        };

        let rows = PrincipalFactsRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            QUERY_B_PRINCIPAL_FACTS,
            [
                workspace.0.into(),
                principal_type.into(),
                principal_id.into(),
                resources_json(resources).into(),
            ],
        ))
        .all(&self.conn)
        .await
        .map_err(db_error)?;

        let [row] = rows.as_slice() else {
            return Err(DomainError::Internal {
                message: "batch principal facts returned an invalid envelope cardinality".into(),
            });
        };

        let facts: StoredPrincipalFacts =
            serde_json::from_value(row.facts.clone()).map_err(|error| DomainError::Internal {
                message: format!("invalid batch principal facts payload: {error}"),
            })?;

        decode_principal_facts(facts, workspace)
    }
}

/// Composition-facing facade preserved for `BatchAuthorizationService` and
/// its existing callers/tests: delegates to the two internal ports, doing the
/// `atlas_core::ids::ResourceRef` conversion at this boundary (D2/S3b).
#[async_trait]
impl BatchAuthorizationSource for PgBatchAuthorizationSource {
    async fn load_subject_facts(
        &self,
        context: &ProjectionAuthContext,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError> {
        self.chain
            .load_subject_facts(context.workspace_id, subjects)
            .await
    }

    async fn load_principal_facts(
        &self,
        context: &ProjectionAuthContext,
        resources: &[ResourceRef],
    ) -> Result<PrincipalFacts, DomainError> {
        let resources = resources
            .iter()
            .map(|resource| {
                atlas_acta::permissions::resource_ref_codec::to_core(resource, context.workspace_id)
            })
            .collect::<Vec<_>>();

        self.facts
            .load_principal_facts(
                atlas_custos::WorkspaceScope(context.workspace_id.0),
                &context.principal,
                &resources,
            )
            .await
    }
}

const QUERY_A_SUBJECT_FACTS: &str = r#"
/* atlas_batch_subject_facts */
WITH RECURSIVE requested AS (
    SELECT ordinal, kind, id
    FROM jsonb_to_recordset($2::jsonb) AS subjects(ordinal bigint, kind text, id uuid)
), subject_targets AS (
    SELECT requested.ordinal, 'documents'::text AS family, documents.id AS document_id, NULL::uuid AS task_id
    FROM requested
    JOIN documents ON requested.kind = 'document'
        AND documents.id = requested.id
        AND documents.workspace_id = $1::uuid
        AND documents.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'tasks'::text, NULL, tasks.id
    FROM requested
    JOIN tasks ON requested.kind = 'task'
        AND tasks.id = requested.id
        AND tasks.workspace_id = $1::uuid
        AND tasks.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'documents'::text, documents.id, NULL
    FROM requested
    JOIN attachments ON requested.kind = 'attachment'
        AND attachments.id = requested.id
        AND attachments.workspace_id = $1::uuid
        AND attachments.deleted_at IS NULL
        AND num_nonnulls(attachments.document_id, attachments.task_id, attachments.comment_id) = 1
    JOIN documents ON documents.id = attachments.document_id
        AND documents.workspace_id = $1::uuid
        AND documents.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'tasks'::text, NULL, tasks.id
    FROM requested
    JOIN attachments ON requested.kind = 'attachment'
        AND attachments.id = requested.id
        AND attachments.workspace_id = $1::uuid
        AND attachments.deleted_at IS NULL
        AND num_nonnulls(attachments.document_id, attachments.task_id, attachments.comment_id) = 1
    JOIN tasks ON tasks.id = attachments.task_id
        AND tasks.workspace_id = $1::uuid
        AND tasks.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'documents'::text, documents.id, NULL
    FROM requested
    JOIN attachments ON requested.kind = 'attachment'
        AND attachments.id = requested.id
        AND attachments.workspace_id = $1::uuid
        AND attachments.deleted_at IS NULL
        AND num_nonnulls(attachments.document_id, attachments.task_id, attachments.comment_id) = 1
    JOIN comments ON comments.id = attachments.comment_id
        AND comments.workspace_id = $1::uuid
        AND comments.deleted_at IS NULL
        AND num_nonnulls(comments.document_id, comments.task_id) = 1
    JOIN documents ON documents.id = comments.document_id
        AND documents.workspace_id = $1::uuid
        AND documents.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'tasks'::text, NULL, tasks.id
    FROM requested
    JOIN attachments ON requested.kind = 'attachment'
        AND attachments.id = requested.id
        AND attachments.workspace_id = $1::uuid
        AND attachments.deleted_at IS NULL
        AND num_nonnulls(attachments.document_id, attachments.task_id, attachments.comment_id) = 1
    JOIN comments ON comments.id = attachments.comment_id
        AND comments.workspace_id = $1::uuid
        AND comments.deleted_at IS NULL
        AND num_nonnulls(comments.document_id, comments.task_id) = 1
    JOIN tasks ON tasks.id = comments.task_id
        AND tasks.workspace_id = $1::uuid
        AND tasks.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'documents'::text, documents.id, NULL
    FROM requested
    JOIN comments ON requested.kind = 'source_comment'
        AND comments.id = requested.id
        AND comments.workspace_id = $1::uuid
        AND comments.deleted_at IS NULL
        AND num_nonnulls(comments.document_id, comments.task_id) = 1
    JOIN documents ON documents.id = comments.document_id
        AND documents.workspace_id = $1::uuid
        AND documents.deleted_at IS NULL
    UNION ALL
    SELECT requested.ordinal, 'tasks'::text, NULL, tasks.id
    FROM requested
    JOIN comments ON requested.kind = 'source_comment'
        AND comments.id = requested.id
        AND comments.workspace_id = $1::uuid
        AND comments.deleted_at IS NULL
        AND num_nonnulls(comments.document_id, comments.task_id) = 1
    JOIN tasks ON tasks.id = comments.task_id
        AND tasks.workspace_id = $1::uuid
        AND tasks.deleted_at IS NULL
), folder_ancestry AS (
    SELECT subject_targets.ordinal, folders.id, folders.parent_folder_id, folders.project_id, 0 AS depth
    FROM subject_targets
    JOIN documents ON documents.id = subject_targets.document_id
    JOIN folders ON folders.id = documents.folder_id
        AND folders.workspace_id = $1::uuid
        AND folders.deleted_at IS NULL
    UNION ALL
    SELECT folder_ancestry.ordinal, folders.id, folders.parent_folder_id, folders.project_id,
           folder_ancestry.depth + 1
    FROM folder_ancestry
    JOIN folders ON folders.id = folder_ancestry.parent_folder_id
        AND folders.workspace_id = $1::uuid
        AND folders.deleted_at IS NULL
    WHERE folder_ancestry.depth < 31
), document_chains AS (
    SELECT subject_targets.ordinal,
           jsonb_build_array(jsonb_build_object('resource', 'acta::document::' || documents.id::text, 'visibility', NULL))
           || COALESCE(folder_rows.folders, '[]'::jsonb)
           || CASE WHEN projects.id IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object(
               'resource', 'acta::project::' || projects.id::text,
               'visibility', jsonb_build_object('visibility', projects.visibility, 'role', projects.visibility_role)
           )) END
           || jsonb_build_array(jsonb_build_object('resource', 'acta::workspace::' || $1::text, 'visibility', NULL)) AS chain
    FROM subject_targets
    JOIN documents ON documents.id = subject_targets.document_id
    LEFT JOIN LATERAL (
        SELECT jsonb_agg(jsonb_build_object('resource', 'acta::folder::' || id::text, 'visibility', NULL) ORDER BY depth) AS folders,
               (array_agg(project_id ORDER BY depth DESC))[1] AS inherited_project_id
        FROM folder_ancestry
        WHERE folder_ancestry.ordinal = subject_targets.ordinal
    ) folder_rows ON true
    LEFT JOIN projects ON projects.id = COALESCE(documents.project_id, folder_rows.inherited_project_id)
        AND projects.workspace_id = $1::uuid
        AND projects.deleted_at IS NULL
    WHERE subject_targets.document_id IS NOT NULL
), task_chains AS (
    SELECT subject_targets.ordinal,
           jsonb_build_array(jsonb_build_object('resource', 'acta::board::' || tasks.board_id::text, 'visibility', NULL))
           || CASE WHEN projects.id IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object(
               'resource', 'acta::project::' || projects.id::text,
               'visibility', jsonb_build_object('visibility', projects.visibility, 'role', projects.visibility_role)
           )) END
           || jsonb_build_array(jsonb_build_object('resource', 'acta::workspace::' || $1::text, 'visibility', NULL)) AS chain
    FROM subject_targets
    JOIN tasks ON tasks.id = subject_targets.task_id
    LEFT JOIN projects ON projects.id = tasks.project_id
        AND projects.workspace_id = $1::uuid
        AND projects.deleted_at IS NULL
    WHERE subject_targets.task_id IS NOT NULL
)
SELECT jsonb_build_object('ordinal', subject_targets.ordinal, 'family', subject_targets.family,
                          'chain', COALESCE(document_chains.chain, task_chains.chain)) AS fact
FROM subject_targets
LEFT JOIN document_chains ON document_chains.ordinal = subject_targets.ordinal
LEFT JOIN task_chains ON task_chains.ordinal = subject_targets.ordinal
ORDER BY subject_targets.ordinal
"#;

const QUERY_B_PRINCIPAL_FACTS: &str = r#"
/* atlas_batch_principal_facts */
WITH requested AS (
    SELECT $1::uuid AS workspace_id, $2::text AS principal_type, $3::uuid AS principal_id
), requested_resources AS (
    SELECT value AS resource_ref
    FROM jsonb_array_elements_text($4::jsonb) AS resources(value)
), principal_users AS (
    SELECT workspace_id, principal_id AS user_id
    FROM requested
    WHERE principal_type = 'user'
    UNION ALL
    SELECT requested.workspace_id, keys.created_by_user_id
    FROM requested
    JOIN custos.api_keys keys ON keys.id = requested.principal_id
    WHERE requested.principal_type = 'api_key'
), user_grants AS (
    SELECT principal_users.user_id,
           COALESCE(jsonb_agg(jsonb_build_object(
               'resource', grants.resource_ref,
               'role', grants.role
           )) FILTER (WHERE grants.id IS NOT NULL), '[]'::jsonb) AS grants
    FROM principal_users
    LEFT JOIN custos.permission_grants grants ON grants.workspace_id = principal_users.workspace_id
        AND (
            grants.user_id = principal_users.user_id
            OR (
                grants.group_id IS NOT NULL
                AND EXISTS (
                    SELECT 1
                    FROM custos.group_members
                    JOIN custos.groups ON groups.id = group_members.group_id
                    WHERE group_members.group_id = grants.group_id
                      AND group_members.user_id = principal_users.user_id
                      AND groups.workspace_id = principal_users.workspace_id
                      AND groups.deleted_at IS NULL
                )
            )
        )
        AND (
            grants.resource_ref = 'acta::workspace::' || principal_users.workspace_id::text
            OR EXISTS (
                SELECT 1 FROM requested_resources
                WHERE requested_resources.resource_ref = grants.resource_ref
            )
        )
    GROUP BY principal_users.user_id
), key_grants AS (
    SELECT requested.principal_id AS key_id,
           COALESCE(jsonb_agg(jsonb_build_object(
               'resource', grants.resource_ref,
               'role', grants.role
           )) FILTER (WHERE grants.id IS NOT NULL), '[]'::jsonb) AS grants
    FROM requested
    LEFT JOIN custos.permission_grants grants ON grants.workspace_id = requested.workspace_id
        AND grants.api_key_id = requested.principal_id
        AND (
            grants.resource_ref = 'acta::workspace::' || requested.workspace_id::text
            OR EXISTS (
                SELECT 1 FROM requested_resources
                WHERE requested_resources.resource_ref = grants.resource_ref
            )
        )
    WHERE requested.principal_type = 'api_key'
    GROUP BY requested.principal_id
), user_facts AS (
    SELECT jsonb_build_object(
        'kind', 'user',
        'user_id', requested.principal_id,
        'is_active', users.id IS NOT NULL AND users.disabled_at IS NULL,
        'effective_membership', memberships.role,
        'is_root_admin', COALESCE(users.is_root OR users.is_system_admin, false),
        'grants', COALESCE(user_grants.grants, '[]'::jsonb)
    ) AS facts
    FROM requested
    LEFT JOIN custos.users ON users.id = requested.principal_id
    LEFT JOIN workspace_memberships memberships
        ON memberships.workspace_id = requested.workspace_id AND memberships.user_id = users.id
    LEFT JOIN user_grants ON user_grants.user_id = users.id
    WHERE requested.principal_type = 'user'
), key_facts AS (
    SELECT jsonb_build_object(
        'kind', 'api_key',
        'key_id', requested.principal_id,
        'is_active', keys.id IS NOT NULL,
        'is_revoked', keys.revoked_at IS NOT NULL,
        'is_expired', keys.expires_at IS NOT NULL AND keys.expires_at <= CURRENT_TIMESTAMP,
        'is_global', COALESCE(keys.is_global, false),
        'scopes', COALESCE(to_jsonb(keys.scopes), '[]'::jsonb),
        'grants', COALESCE(key_grants.grants, '[]'::jsonb),
        'creator', jsonb_build_object(
            'user_id', creator.id,
            'is_active', creator.id IS NOT NULL AND creator.disabled_at IS NULL,
            'effective_membership', creator_membership.role,
            'is_root_admin', COALESCE(creator.is_root OR creator.is_system_admin, false),
            'grants', COALESCE(user_grants.grants, '[]'::jsonb)
        )
    ) AS facts
    FROM requested
    LEFT JOIN custos.api_keys keys ON keys.id = requested.principal_id
    LEFT JOIN custos.users creator ON creator.id = keys.created_by_user_id
    LEFT JOIN workspace_memberships creator_membership
        ON creator_membership.workspace_id = requested.workspace_id
        AND creator_membership.user_id = creator.id
    LEFT JOIN key_grants ON key_grants.key_id = keys.id
    LEFT JOIN user_grants ON user_grants.user_id = creator.id
    WHERE requested.principal_type = 'api_key'
)
SELECT facts FROM user_facts
UNION ALL
SELECT facts FROM key_facts
"#;

/// Encodes the requested resource set as an array of canonical `resource_ref`
/// strings (`acta::<kind>::<uuid>`), the shape `QUERY_B_PRINCIPAL_FACTS`'s
/// `requested_resources` CTE consumes directly. The set always includes the
/// workspace-scope ref (every resolved chain ends in `ResourceRef::Workspace`,
/// per `validate_subject_chain`), so a workspace-scope grant equality-matches
/// without a separate "matches everything" branch in SQL.
fn resources_json(resources: &[atlas_core::ids::ResourceRef]) -> serde_json::Value {
    serde_json::Value::Array(
        resources
            .iter()
            .map(|resource| serde_json::Value::String(resource.to_string()))
            .collect(),
    )
}

fn subjects_json(subjects: &[ProjectionSubject]) -> serde_json::Value {
    serde_json::Value::Array(
        subjects
            .iter()
            .enumerate()
            .map(|(ordinal, subject)| {
                let (kind, id) = match subject {
                    ProjectionSubject::Document(id) => ("document", *id),
                    ProjectionSubject::Task(id) => ("task", *id),
                    ProjectionSubject::Attachment(id) => ("attachment", *id),
                    ProjectionSubject::SourceComment(id) => ("source_comment", *id),
                };
                serde_json::json!({ "ordinal": ordinal, "kind": kind, "id": id })
            })
            .collect(),
    )
}

fn decode_subject_fact(
    fact: StoredSubjectFact,
    workspace: WorkspaceId,
) -> Result<SubjectFact, DomainError> {
    let family = match fact.family.as_str() {
        "documents" => SubjectFamily::Documents,
        "tasks" => SubjectFamily::Tasks,
        _ => {
            return Err(DomainError::Internal {
                message: "batch subject facts contained an invalid subject family".into(),
            });
        }
    };

    let chain = ResourceChain {
        segments: fact
            .chain
            .into_iter()
            .map(|segment| decode_chain_segment(segment, workspace))
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_subject_chain(&chain, family)?;

    Ok(SubjectFact {
        ordinal: fact.ordinal,
        chain,
        family,
    })
}

fn decode_chain_segment(
    segment: StoredChainSegment,
    workspace: WorkspaceId,
) -> Result<ChainSegment, DomainError> {
    let resource = decode_resource(&segment.resource, workspace)?;
    let visibility = segment.visibility.map(decode_visibility).transpose()?;

    Ok(ChainSegment {
        resource,
        visibility,
    })
}

fn decode_visibility(visibility: StoredVisibility) -> Result<Visibility, DomainError> {
    if visibility.visibility == "private" {
        if visibility
            .role
            .as_deref()
            .is_some_and(|role| role != "viewer" && role != "editor")
        {
            return Err(DomainError::Internal {
                message: "batch subject facts contained an invalid visibility role".into(),
            });
        }
        return Ok(Visibility::Private);
    }

    let role = match visibility.role.as_deref() {
        Some("viewer") => VisibilityRole::Viewer,
        Some("editor") => VisibilityRole::Editor,
        _ => {
            return Err(DomainError::Internal {
                message: "batch subject facts contained an invalid visibility role".into(),
            });
        }
    };

    match visibility.visibility.as_str() {
        "workspace" => Ok(Visibility::Workspace(role)),
        "public" => Ok(Visibility::Public(role)),
        _ => Err(DomainError::Internal {
            message: "batch subject facts contained an invalid visibility".into(),
        }),
    }
}

fn validate_subject_chain(chain: &ResourceChain, family: SubjectFamily) -> Result<(), DomainError> {
    let Some((last, prefixes)) = chain.segments.split_last() else {
        return Err(invalid_subject_chain());
    };
    if last.resource != ResourceRef::Workspace || last.visibility.is_some() {
        return Err(invalid_subject_chain());
    }

    let mut seen = BTreeSet::new();
    for segment in &chain.segments {
        if !seen.insert(segment.resource.clone()) {
            return Err(invalid_subject_chain());
        }
    }

    match family {
        SubjectFamily::Documents => {
            let Some((document, rest)) = prefixes.split_first() else {
                return Err(invalid_subject_chain());
            };
            if !matches!(document.resource, ResourceRef::Document(_))
                || document.visibility.is_some()
            {
                return Err(invalid_subject_chain());
            }
            let (folders, project) = split_document_chain(rest);
            if folders.len() > 32
                || folders.iter().any(|segment| {
                    !matches!(segment.resource, ResourceRef::Folder(_))
                        || segment.visibility.is_some()
                })
                || project.is_some_and(|segment| !is_project_segment(segment))
            {
                return Err(invalid_subject_chain());
            }
        }
        SubjectFamily::Tasks => {
            let Some((board, rest)) = prefixes.split_first() else {
                return Err(invalid_subject_chain());
            };
            if !matches!(board.resource, ResourceRef::Board(_)) || board.visibility.is_some() {
                return Err(invalid_subject_chain());
            }
            if rest.len() > 1
                || rest
                    .first()
                    .is_some_and(|segment| !is_project_segment(segment))
            {
                return Err(invalid_subject_chain());
            }
        }
    }

    Ok(())
}

fn split_document_chain(segments: &[ChainSegment]) -> (&[ChainSegment], Option<&ChainSegment>) {
    let Some((last, prefixes)) = segments.split_last() else {
        return (segments, None);
    };

    if matches!(last.resource, ResourceRef::Project(_)) {
        (prefixes, Some(last))
    } else {
        (segments, None)
    }
}

fn is_project_segment(segment: &ChainSegment) -> bool {
    matches!(segment.resource, ResourceRef::Project(_)) && segment.visibility.is_some()
}

fn invalid_subject_chain() -> DomainError {
    DomainError::Internal {
        message: "batch subject facts contained a noncanonical resource chain".into(),
    }
}

fn decode_principal_facts(
    facts: StoredPrincipalFacts,
    workspace: WorkspaceId,
) -> Result<PrincipalFacts, DomainError> {
    match facts {
        StoredPrincipalFacts::User(facts) => {
            Ok(PrincipalFacts::User(decode_user_facts(facts, workspace)?))
        }
        StoredPrincipalFacts::ApiKey(facts) => Ok(PrincipalFacts::ApiKey(ApiKeyFacts {
            key_id: ApiKeyId(facts.key_id),
            is_active: facts.is_active,
            is_revoked: facts.is_revoked,
            is_expired: facts.is_expired,
            is_global: facts.is_global,
            scopes: decode_scopes(facts.scopes)?,
            grants: decode_grants(facts.grants, workspace)?,
            creator: decode_user_facts(facts.creator, workspace)?,
        })),
    }
}

fn decode_user_facts(
    facts: StoredUserFacts,
    workspace: WorkspaceId,
) -> Result<UserFacts, DomainError> {
    Ok(UserFacts {
        user_id: UserId(facts.user_id),
        is_active: facts.is_active,
        effective_membership: facts
            .effective_membership
            .as_deref()
            .map(decode_membership)
            .transpose()?,
        is_root_admin: facts.is_root_admin,
        grants: decode_grants(facts.grants, workspace)?,
    })
}

fn decode_membership(role: &str) -> Result<MemberRole, DomainError> {
    match role {
        "owner" => Ok(MemberRole::Owner),
        "admin" => Ok(MemberRole::Admin),
        "member" => Ok(MemberRole::Member),
        _ => Err(DomainError::Internal {
            message: "batch principal facts contained an invalid membership role".into(),
        }),
    }
}

fn decode_grants(
    grants: Vec<StoredGrant>,
    workspace: WorkspaceId,
) -> Result<Vec<(ResourceRef, ResourceRole)>, DomainError> {
    let mut decoded = Vec::with_capacity(grants.len());
    let mut resources = BTreeSet::new();

    for grant in grants {
        let resource = decode_resource(&grant.resource, workspace)?;
        if !resources.insert(resource.clone()) {
            return Err(DomainError::Internal {
                message: "batch principal facts contained duplicate grants".into(),
            });
        }
        decoded.push((resource, decode_role(&grant.role)?));
    }

    Ok(decoded)
}

fn decode_scopes(scopes: Vec<String>) -> Result<Vec<Capability>, DomainError> {
    let mut decoded = Vec::with_capacity(scopes.len());
    let mut seen = BTreeSet::new();

    for scope in scopes {
        if !seen.insert(scope.clone()) {
            return Err(DomainError::Internal {
                message: "batch principal facts contained duplicate key scopes".into(),
            });
        }
        decoded.push(scope.parse().map_err(|_| DomainError::Internal {
            message: "batch principal facts contained an unknown key capability".into(),
        })?);
    }

    Ok(decoded)
}

fn decode_resource(resource_ref: &str, workspace: WorkspaceId) -> Result<ResourceRef, DomainError> {
    let core: atlas_core::ids::ResourceRef =
        resource_ref.parse().map_err(|_| DomainError::Internal {
            message: "batch principal facts contained an invalid grant resource".into(),
        })?;

    atlas_acta::permissions::resource_ref_codec::from_core(&core, workspace)
}

fn decode_role(role: &str) -> Result<ResourceRole, DomainError> {
    match role {
        "viewer" => Ok(ResourceRole::Viewer),
        "editor" => Ok(ResourceRole::Editor),
        "admin" => Ok(ResourceRole::Admin),
        _ => Err(DomainError::Internal {
            message: "batch principal facts contained an invalid grant role".into(),
        }),
    }
}

/// Applies the existing permission resolver to a bounded heterogeneous batch.
///
/// The source is deliberately split into exactly two bulk operations. Missing
/// subject facts are denied; source failures propagate instead of becoming a
/// denial sentinel so callers can fail the whole projection safely.
pub(crate) struct BatchAuthorizationService<S> {
    source: S,
}

impl<S> BatchAuthorizationService<S>
where
    S: BatchAuthorizationSource,
{
    pub(crate) fn new(source: S) -> Self {
        Self { source }
    }

    pub(crate) async fn authorize(
        &self,
        context: &ProjectionAuthContext,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<bool>, DomainError> {
        if subjects.is_empty() {
            return Ok(Vec::new());
        }

        let facts = self.source.load_subject_facts(context, subjects).await?;
        validate_subject_facts(&facts, subjects)?;
        let resources = distinct_resources(&facts);
        let principal_facts = self
            .source
            .load_principal_facts(context, &resources)
            .await?;
        validate_principal_facts(context, &principal_facts)?;

        let mut decisions = vec![false; subjects.len()];
        for fact in facts {
            let decision =
                decisions
                    .get_mut(fact.ordinal)
                    .ok_or_else(|| DomainError::Internal {
                        message: "batch authorization source returned an invalid ordinal".into(),
                    })?;
            *decision = authorize_fact(context, &principal_facts, &fact);
        }

        Ok(decisions)
    }
}

fn validate_subject_facts(
    facts: &[SubjectFact],
    subjects: &[ProjectionSubject],
) -> Result<(), DomainError> {
    let mut seen_ordinals = vec![false; subjects.len()];

    for fact in facts {
        let Some(subject) = subjects.get(fact.ordinal) else {
            return Err(DomainError::Internal {
                message: "batch authorization source returned an invalid ordinal".into(),
            });
        };

        let seen = seen_ordinals
            .get_mut(fact.ordinal)
            .ok_or_else(|| DomainError::Internal {
                message: "batch authorization source returned an invalid ordinal".into(),
            })?;

        if *seen {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate subject facts".into(),
            });
        }

        if !subject_matches_family(*subject, fact.family) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned facts for the wrong subject family"
                    .into(),
            });
        }

        *seen = true;
    }

    Ok(())
}

fn subject_matches_family(subject: ProjectionSubject, family: SubjectFamily) -> bool {
    matches!(
        (subject, family),
        (ProjectionSubject::Document(_), SubjectFamily::Documents)
            | (ProjectionSubject::Task(_), SubjectFamily::Tasks)
            | (
                ProjectionSubject::Attachment(_),
                SubjectFamily::Documents | SubjectFamily::Tasks
            )
            | (
                ProjectionSubject::SourceComment(_),
                SubjectFamily::Documents | SubjectFamily::Tasks
            )
    )
}

fn distinct_resources(facts: &[SubjectFact]) -> Vec<ResourceRef> {
    let mut resources = BTreeSet::new();

    for fact in facts {
        for segment in &fact.chain.segments {
            resources.insert(segment.resource.clone());
        }
    }

    resources.into_iter().collect()
}

fn authorize_fact(
    context: &ProjectionAuthContext,
    principal_facts: &PrincipalFacts,
    fact: &SubjectFact,
) -> bool {
    let role = match principal_facts {
        PrincipalFacts::User(facts) => resolve_user_role(&context.principal, facts, &fact.chain),
        PrincipalFacts::ApiKey(facts) => resolve_api_key_role(context, facts, &fact.chain),
    };

    role.is_some_and(|role| role >= ResourceRole::Viewer)
        && api_key_has_read_capability(principal_facts, fact.family)
}

fn resolve_user_role(
    principal: &Principal,
    facts: &UserFacts,
    chain: &ResourceChain,
) -> Option<ResourceRole> {
    if !facts.is_active || (!facts.is_root_admin && facts.effective_membership.is_none()) {
        return None;
    }

    let membership = if facts.is_root_admin {
        Some(MemberRole::Admin)
    } else {
        facts.effective_membership.clone()
    };

    resolve(&ResolutionInput {
        principal,
        membership,
        chain,
        grants: &facts.grants,
    })
}

fn resolve_api_key_role(
    context: &ProjectionAuthContext,
    facts: &ApiKeyFacts,
    chain: &ResourceChain,
) -> Option<ResourceRole> {
    if !facts.is_active || facts.is_revoked || facts.is_expired {
        return None;
    }

    let creator = Principal::User(facts.creator.user_id);
    let creator_role = resolve_user_role(&creator, &facts.creator, chain);

    let role = if facts.is_global {
        creator_role
    } else {
        let key_role = resolve(&ResolutionInput {
            principal: &context.principal,
            membership: None,
            chain,
            grants: &facts.grants,
        });
        match (key_role, creator_role) {
            (Some(key_role), Some(creator_role)) => Some(key_role.min(creator_role)),
            _ => None,
        }
    };

    role.map(|role| role.min(ResourceRole::Editor))
}

fn api_key_has_read_capability(principal_facts: &PrincipalFacts, family: SubjectFamily) -> bool {
    let PrincipalFacts::ApiKey(facts) = principal_facts else {
        return true;
    };

    let family = match family {
        SubjectFamily::Documents => CapabilityFamily::Docs,
        SubjectFamily::Tasks => CapabilityFamily::Tasks,
    };
    let capability = Capability {
        family,
        action: CapabilityAction::Read,
    };

    facts.scopes.contains(&capability)
}

fn validate_principal_facts(
    context: &ProjectionAuthContext,
    facts: &PrincipalFacts,
) -> Result<(), DomainError> {
    match (&context.principal, facts) {
        (Principal::User(context_id), PrincipalFacts::User(facts))
            if *context_id == facts.user_id =>
        {
            validate_grants(&facts.grants)
        }
        (Principal::ApiKey(context_id), PrincipalFacts::ApiKey(facts))
            if *context_id == facts.key_id =>
        {
            validate_grants(&facts.grants)?;
            validate_user_facts(&facts.creator)?;
            validate_scopes(&facts.scopes)
        }
        _ => Err(DomainError::Internal {
            message: "batch authorization source returned facts for the wrong principal".into(),
        }),
    }
}

fn validate_user_facts(facts: &UserFacts) -> Result<(), DomainError> {
    validate_grants(&facts.grants)
}

fn validate_grants(grants: &[(ResourceRef, ResourceRole)]) -> Result<(), DomainError> {
    let mut seen = BTreeSet::new();
    for (resource, role) in grants {
        if !seen.insert(resource.clone()) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate grants".into(),
            });
        }
        let _ = role;
    }
    Ok(())
}

fn validate_scopes(scopes: &[Capability]) -> Result<(), DomainError> {
    for (index, scope) in scopes.iter().enumerate() {
        if scopes.iter().take(index).any(|previous| previous == scope) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate key scopes".into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod distinct_resources_tests {
    use super::*;
    use atlas_acta::ids::BoardId;
    use atlas_acta::ids::DocumentId;
    use atlas_acta::ids::FolderId;
    use atlas_acta::ids::ProjectId;

    fn segment(resource: ResourceRef) -> ChainSegment {
        ChainSegment {
            resource,
            visibility: None,
        }
    }

    #[test]
    fn distinct_resources_dedupes_and_orders_a_mixed_chain_set() {
        let project = ProjectId(Uuid::now_v7());
        let folder = FolderId(Uuid::now_v7());
        let document = DocumentId(Uuid::now_v7());
        let board = BoardId(Uuid::now_v7());

        let facts = vec![
            SubjectFact {
                ordinal: 0,
                chain: ResourceChain {
                    segments: vec![
                        segment(ResourceRef::Document(document)),
                        segment(ResourceRef::Folder(folder)),
                        segment(ResourceRef::Project(project)),
                        segment(ResourceRef::Workspace),
                    ],
                },
                family: SubjectFamily::Documents,
            },
            SubjectFact {
                ordinal: 1,
                chain: ResourceChain {
                    segments: vec![
                        segment(ResourceRef::Board(board)),
                        segment(ResourceRef::Project(project)),
                        segment(ResourceRef::Workspace),
                    ],
                },
                family: SubjectFamily::Tasks,
            },
        ];

        let resources = distinct_resources(&facts);

        assert_eq!(
            resources,
            vec![
                ResourceRef::Workspace,
                ResourceRef::Project(project),
                ResourceRef::Folder(folder),
                ResourceRef::Document(document),
                ResourceRef::Board(board),
            ],
            "duplicate Workspace and Project segments across chains must collapse to one \
             entry each, and the result must follow ResourceRef's derived variant order"
        );
    }
}

#[cfg(test)]
mod membership_join_debt_tests {
    use super::QUERY_B_PRINCIPAL_FACTS;

    /// D3: `PrincipalFactsSource` keeps `QUERY_B`'s `workspace_memberships`
    /// join for S3, as named deferred debt for E4's `ResourceProvider::
    /// members_of`. Moving the join to the Acta side is impossible in two
    /// statements here — the creator user id is only known after Custos
    /// answers, so a clean cut would cost a third round trip on the agent hot
    /// path. This test pins that the join still lives in `QUERY_B`, once for
    /// the principal's own membership and once for an api key's creator, so
    /// a future removal of either join is a deliberate, reviewed change, not
    /// an accidental regression.
    #[test]
    fn query_b_still_joins_workspace_memberships_as_named_e4_debt() {
        let membership_joins = QUERY_B_PRINCIPAL_FACTS
            .matches("workspace_memberships")
            .count();

        assert_eq!(
            membership_joins, 2,
            "QUERY_B_PRINCIPAL_FACTS must still join workspace_memberships exactly twice \
             (principal membership, api-key creator membership) — this is deferred to E4's \
             ResourceProvider::members_of (D3), not resolved by the S3b3 port cut"
        );
    }
}
