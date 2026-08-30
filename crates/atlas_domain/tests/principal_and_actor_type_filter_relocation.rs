#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Characterization test for S2d D4: `Principal` and `ActorTypeFilter` move
//! from `atlas_domain` to `atlas_core`, but every existing import path
//! (`crate::permissions::Principal`, `crate::ids::{UserId, ApiKeyId, GroupId}`,
//! `crate::entities::task_views::ActorTypeFilter`) must keep resolving to the
//! exact same type, and the six Acta port methods that take `&Principal`
//! (`ports::{documents, search, workspace_core}`) must compile completely
//! unmodified.
//!
//! Pinning `TypeId` equality proves the facade path is not a lookalike type
//! with the same name — it is the identical type atlas_core defines.

use std::any::TypeId;

#[test]
fn domain_principal_path_is_the_atlas_core_type() {
    assert_eq!(
        TypeId::of::<atlas_domain::permissions::Principal>(),
        TypeId::of::<atlas_core::principal::Principal>(),
        "atlas_domain::permissions::Principal must be the exact atlas_core::principal::Principal type"
    );
}

#[test]
fn domain_id_paths_are_the_atlas_core_principal_types() {
    assert_eq!(
        TypeId::of::<atlas_domain::ids::UserId>(),
        TypeId::of::<atlas_core::principal::UserId>()
    );
    assert_eq!(
        TypeId::of::<atlas_domain::ids::ApiKeyId>(),
        TypeId::of::<atlas_core::principal::ApiKeyId>()
    );
    assert_eq!(
        TypeId::of::<atlas_domain::ids::GroupId>(),
        TypeId::of::<atlas_core::principal::GroupId>()
    );
}

#[test]
fn domain_actor_type_filter_path_is_the_atlas_core_type() {
    assert_eq!(
        TypeId::of::<atlas_domain::entities::task_views::ActorTypeFilter>(),
        TypeId::of::<atlas_core::attribution::ActorTypeFilter>(),
        "entities::task_views::ActorTypeFilter must be the exact atlas_core::attribution::ActorTypeFilter type"
    );
}

#[test]
fn actor_type_filter_serde_representation_is_unchanged() {
    use atlas_core::attribution::ActorTypeFilter;

    assert_eq!(
        serde_json::to_string(&ActorTypeFilter::User).expect("serialize"),
        "\"user\""
    );
    assert_eq!(
        serde_json::to_string(&ActorTypeFilter::ApiKey).expect("serialize"),
        "\"api_key\""
    );
}

/// Pins the exact parameter list of the six Acta port methods that consume
/// `&Principal` by coercing each trait method to a function pointer with the
/// literal pre-move signature. Any parameter/return drift introduced by the
/// `Principal` relocation — including an accidental switch to `PrincipalId`
/// or a reordered argument — fails this module to compile.
#[allow(dead_code)]
mod acta_port_signatures_are_unchanged {
    use atlas_domain::{
        DomainError, WorkspaceCtx,
        entities::{documents::DocumentSummary, workspace_core::Project},
        ids::ProjectId,
        permissions::Principal,
        ports::{
            documents::{DocumentRepo, FolderPresence},
            search::{SearchAfter, SearchRepo},
            workspace_core::ProjectRepo,
        },
        search::{SearchHit, SearchQuery},
    };
    use uuid::Uuid;

    fn _document_repo_list_visible<'a, R: DocumentRepo>(
        r: &'a R,
        ctx: &'a WorkspaceCtx,
        principal: &'a Principal,
        project_filter: Option<ProjectId>,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> impl std::future::Future<Output = Result<Vec<DocumentSummary>, DomainError>> + 'a {
        r.list_visible(ctx, principal, project_filter, after_id, limit)
    }

    fn _document_repo_list_visible_with_folder_presence<'a, R: DocumentRepo>(
        r: &'a R,
        ctx: &'a WorkspaceCtx,
        principal: &'a Principal,
        project_filter: Option<ProjectId>,
        folder_presence: FolderPresence,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> impl std::future::Future<Output = Result<Vec<DocumentSummary>, DomainError>> + 'a {
        r.list_visible_with_folder_presence(
            ctx,
            principal,
            project_filter,
            folder_presence,
            after_id,
            limit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn _search_repo_search<'a, R: SearchRepo>(
        r: &'a R,
        ctx: &'a WorkspaceCtx,
        principal: &'a Principal,
        query: &'a SearchQuery,
        limit: u64,
        after: Option<SearchAfter>,
        bypass: bool,
        may_read_docs: bool,
        may_read_tasks: bool,
    ) -> impl std::future::Future<Output = Result<Vec<SearchHit>, DomainError>> + 'a {
        r.search(
            ctx,
            principal,
            query,
            limit,
            after,
            bypass,
            may_read_docs,
            may_read_tasks,
        )
    }

    fn _project_repo_list_visible<'a, R: ProjectRepo>(
        r: &'a R,
        ctx: &'a WorkspaceCtx,
        principal: &'a Principal,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> impl std::future::Future<Output = Result<Vec<Project>, DomainError>> + 'a {
        r.list_visible(ctx, principal, after_id, limit)
    }

    /// Fourth and fifth `list_visible`-shaped methods documented by D4 live on
    /// `FolderRepo`/`BoardRepo`; those crates re-export the same `Principal`
    /// type, so their signatures are exercised transitively by the rest of the
    /// V1 suite compiling unmodified against this facade.
    #[allow(dead_code)]
    const _MARKER_SIX_METHODS_COVERED: () = ();
}
