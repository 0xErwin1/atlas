#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::{Router, middleware as axum_middleware, routing::get};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{
    classify::ServerErrorsFailureClass,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod auth;
pub mod authz;
pub mod config;
pub mod crypto;
pub mod dispatcher;
pub mod embeddings;
pub mod error;
pub mod live;
pub mod middleware;
pub mod persistence;
pub mod presence;
pub mod routes;
pub mod semantic_indexer;
pub mod services;
pub mod state;
pub mod webhook_url;

/// Test-only server assembly for the desktop integration gate.
#[cfg(feature = "desktop-gate-support")]
pub mod desktop_gate_support {
    use crate::persistence::repos::{
        MembershipRepo, NewUser, NewWorkspace, PgMembershipRepo, PgSessionRepo, PgUserRepo,
        PgWorkspaceRepo, SessionRepo, UserRepo, WorkspaceRepo,
    };
    use atlas_domain::{
        Actor, WorkspaceCtx,
        entities::identity::MemberRole,
        ids::{UserId, WorkspaceId},
    };
    use sea_orm::{ConnectionTrait, DatabaseConnection};

    pub use crate::app;
    pub use crate::state::AppState;

    /// Builds the deterministic server state used by the desktop gate.
    pub async fn app_state(db: DatabaseConnection) -> Result<AppState, anyhow::Error> {
        AppState::for_test(db).await
    }

    /// Identity material generated only for the in-process desktop gate server.
    pub struct EphemeralIdentity {
        user_id: UserId,
        pub username: String,
        pub password: String,
        pub workspace_slug: String,
        pub workspace_id: uuid::Uuid,
    }

    /// Creates an activated workspace owner with credentials confined to the caller.
    pub async fn seed_ephemeral_identity(
        db: &DatabaseConnection,
    ) -> Result<EphemeralIdentity, anyhow::Error> {
        let suffix = uuid::Uuid::now_v7().as_simple().to_string();
        let username = format!("gate-{suffix}");
        let password = format!("Gate{suffix}Aa!");
        let password_hash = crate::auth::password::hash(password.clone())
            .await
            .map_err(|_| anyhow::anyhow!("gate identity password setup failed"))?;

        let user = PgUserRepo { conn: db.clone() }
            .create(NewUser {
                username: username.clone(),
                display_name: username.clone(),
                email: None,
                password_hash: Some(password_hash),
                is_root: false,
                is_system_admin: false,
            })
            .await?;

        db.execute_unprepared(&format!(
            "UPDATE users SET activated_at = now() WHERE id = '{}'",
            user.id.0
        ))
        .await?;

        let workspace_slug = format!("ws-{suffix}");
        let workspace = PgWorkspaceRepo { conn: db.clone() }
            .create(NewWorkspace {
                id: WorkspaceId::new(),
                name: format!("Workspace {suffix}"),
                slug: workspace_slug.clone(),
            })
            .await?;
        let ctx = WorkspaceCtx::new(workspace.id, Actor::User(user.id));

        PgMembershipRepo { conn: db.clone() }
            .add(&ctx, user.id, MemberRole::Owner)
            .await?;

        Ok(EphemeralIdentity {
            user_id: user.id,
            username,
            password,
            workspace_slug,
            workspace_id: workspace.id.0,
        })
    }

    /// Revokes only the generated gate user's active sessions through the test-only server seam.
    pub async fn revoke_ephemeral_sessions(
        db: &DatabaseConnection,
        identity: &EphemeralIdentity,
    ) -> Result<(), anyhow::Error> {
        PgSessionRepo { conn: db.clone() }
            .revoke_all_for_user(identity.user_id)
            .await?;
        Ok(())
    }
}

use crate::state::AppState;

/// Builds the full application router with all routes and the middleware stack.
pub fn app(state: AppState) -> Router {
    // burst_size(5) and per_second(1) are non-zero, so finish() always returns Some here.
    #[allow(clippy::expect_used)]
    let login_config = {
        let mut b = GovernorConfigBuilder::default();
        let cfg = b
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("governor config");
        std::sync::Arc::new(cfg)
    };

    // Total-body cap for the attachment upload route. The route previously used
    // `DefaultBodyLimit::disable()`, so the only guard was the per-chunk streaming
    // cap inside the handler's `file` part — any other multipart part streamed
    // unbounded and pinned a worker. Re-apply a hard total-body limit sized to the
    // per-file cap plus slack for multipart boundaries/headers and any extra parts,
    // so no part can stream without bound while the 20 MiB per-file cap still holds.
    const ATTACHMENT_BODY_SLACK: u64 = 1024 * 1024;
    let attachment_body_limit = usize::try_from(
        state
            .max_attachment_bytes
            .saturating_add(ATTACHMENT_BODY_SLACK),
    )
    .unwrap_or(usize::MAX);

    let protected = Router::new()
        .route(
            "/api/auth/logout",
            axum::routing::post(routes::auth::logout),
        )
        .route("/api/auth/me", get(routes::auth::me))
        .route(
            "/api/auth/change-password",
            axum::routing::post(routes::auth::change_password),
        )
        // Self-service profile (any authenticated user)
        .route(
            "/api/users/me",
            axum::routing::patch(routes::auth::update_me),
        )
        // Self-service UI state (human users only; agents are rejected at the handler)
        .route(
            "/api/me/ui-state",
            get(routes::ui_state::get_ui_state).put(routes::ui_state::set_ui_state),
        )
        // Server metadata (any authenticated principal)
        .route("/api/meta", get(routes::health::meta))
        // Users (root-only)
        .route(
            "/api/users",
            axum::routing::post(routes::users::create_user).get(routes::users::list_users),
        )
        .route(
            "/api/users/{user_id}/disable",
            axum::routing::post(routes::users::disable_user),
        )
        .route(
            "/api/users/{user_id}/enable",
            axum::routing::post(routes::users::enable_user),
        )
        .route(
            "/api/users/{user_id}/reset-password",
            axum::routing::post(routes::users::reset_password),
        )
        .route(
            "/api/users/{user_id}/activation-link",
            axum::routing::post(routes::users::regenerate_activation_link),
        )
        .route(
            "/api/users/{user_id}/system-admin",
            axum::routing::post(routes::users::set_system_admin),
        )
        .route(
            "/api/users/{user_id}/memberships",
            get(routes::users::list_user_memberships),
        )
        // Workspace
        .route(
            "/api/workspaces",
            get(routes::workspaces::list_workspaces).post(routes::workspaces::create_workspace),
        )
        .route(
            "/api/workspaces/{ws}",
            get(routes::workspaces::get_workspace).patch(routes::workspaces::update_workspace),
        )
        // Admin workspace list (root-only)
        .route(
            "/api/admin/workspaces",
            get(routes::workspaces::admin_list_workspaces),
        )
        // Admin workspace mutate (root-only): re-slug and soft-delete
        .route(
            "/api/admin/workspaces/{ws}",
            axum::routing::patch(routes::workspaces::admin_update_workspace)
                .delete(routes::workspaces::admin_delete_workspace),
        )
        // Admin security audit log (root/system_admin only)
        .route("/api/admin/audit", get(routes::audit::list_platform_audit))
        // API keys — top-level (user-owned, workspace-independent)
        .route(
            "/api/api-keys",
            axum::routing::post(routes::api_keys::create_user_api_key)
                .get(routes::api_keys::list_user_api_keys),
        )
        .route(
            "/api/api-keys/{key_id}",
            axum::routing::delete(routes::api_keys::revoke_user_api_key)
                .patch(routes::api_keys::update_user_api_key),
        )
        .route(
            "/api/api-keys/{key_id}/grants",
            axum::routing::get(routes::api_keys::list_api_key_grants),
        )
        .route(
            "/api/api-keys/{key_id}/grants/{grant_id}",
            axum::routing::delete(routes::api_keys::delete_api_key_grant),
        )
        // Projects
        .route(
            "/api/workspaces/{ws}/projects",
            axum::routing::post(routes::projects::create_project)
                .get(routes::projects::list_projects),
        )
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}",
            get(routes::projects::get_project)
                .patch(routes::projects::update_project)
                .delete(routes::projects::delete_project),
        )
        // Project grants
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}/grants",
            axum::routing::post(routes::grants::create_project_grant)
                .get(routes::grants::list_project_grants),
        )
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
            axum::routing::delete(routes::grants::delete_project_grant),
        )
        // Workspace grants
        .route(
            "/api/workspaces/{ws}/grants",
            axum::routing::post(routes::grants::create_workspace_grant)
                .get(routes::grants::list_workspace_grants),
        )
        .route(
            "/api/workspaces/{ws}/grants/{grant_id}",
            axum::routing::delete(routes::grants::delete_workspace_grant),
        )
        // Workspace members (principals addressable by a grant)
        .route(
            "/api/workspaces/{ws}/members",
            get(routes::members::list_workspace_members).post(routes::members::add_member),
        )
        .route(
            "/api/workspaces/{ws}/assignable-users",
            get(routes::members::list_assignable_users),
        )
        .route(
            "/api/workspaces/{ws}/members/{user_id}",
            axum::routing::patch(routes::members::update_member_role)
                .delete(routes::members::remove_member),
        )
        // Groups (workspace principal groups)
        .route(
            "/api/workspaces/{ws}/groups",
            axum::routing::post(routes::groups::create_group).get(routes::groups::list_groups),
        )
        .route(
            "/api/workspaces/{ws}/groups/{group_id}",
            axum::routing::delete(routes::groups::delete_group),
        )
        .route(
            "/api/workspaces/{ws}/groups/{group_id}/members",
            axum::routing::post(routes::groups::add_group_member)
                .get(routes::groups::list_group_members),
        )
        .route(
            "/api/workspaces/{ws}/groups/{group_id}/members/{user_id}",
            axum::routing::delete(routes::groups::remove_group_member),
        )
        // Tags (workspace tag registry)
        .route(
            "/api/workspaces/{ws}/tags",
            axum::routing::get(routes::tags::list_tags).post(routes::tags::create_tag),
        )
        .route(
            "/api/workspaces/{ws}/tags/used",
            axum::routing::get(routes::tags::list_used_labels),
        )
        .route(
            "/api/workspaces/{ws}/tags/{tag_id}",
            axum::routing::patch(routes::tags::patch_tag).delete(routes::tags::delete_tag),
        )
        // Status templates (workspace default-status registry)
        .route(
            "/api/workspaces/{ws}/status-templates",
            axum::routing::get(routes::status_templates::list_status_templates)
                .post(routes::status_templates::create_status_template),
        )
        .route(
            "/api/workspaces/{ws}/status-templates/{template_id}",
            axum::routing::patch(routes::status_templates::update_status_template)
                .delete(routes::status_templates::delete_status_template),
        )
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/apply-status-templates",
            axum::routing::post(routes::status_templates::apply_status_templates),
        )
        // Property definitions (workspace custom-field registry)
        .route(
            "/api/workspaces/{ws}/property-definitions",
            axum::routing::get(routes::property_definitions::list_property_definitions)
                .post(routes::property_definitions::create_property_definition),
        )
        .route(
            "/api/workspaces/{ws}/property-definitions/{property_definition_id}",
            axum::routing::delete(routes::property_definitions::delete_property_definition),
        )
        // Saved searches (per-owner personal search registry)
        .route(
            "/api/workspaces/{ws}/saved-searches",
            axum::routing::get(routes::saved_searches::list_saved_searches)
                .post(routes::saved_searches::create_saved_search),
        )
        .route(
            "/api/workspaces/{ws}/saved-searches/{id}",
            axum::routing::patch(routes::saved_searches::rename_saved_search)
                .delete(routes::saved_searches::delete_saved_search),
        )
        // Task views (per-owner personal filter views)
        .route(
            "/api/workspaces/{ws}/task-views",
            axum::routing::get(routes::task_views::list_task_views)
                .post(routes::task_views::create_task_view),
        )
        .route(
            "/api/workspaces/{ws}/task-views/{id}",
            axum::routing::get(routes::task_views::get_task_view)
                .patch(routes::task_views::update_task_view)
                .delete(routes::task_views::delete_task_view),
        )
        // Boards
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}/boards",
            axum::routing::post(routes::boards::create_board).get(routes::boards::list_boards),
        )
        .route(
            "/api/workspaces/{ws}/boards/{board_id}",
            axum::routing::get(routes::boards::get_board)
                .patch(routes::boards::update_board)
                .delete(routes::boards::delete_board),
        )
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/move",
            axum::routing::patch(routes::boards::move_board),
        )
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/columns",
            axum::routing::post(routes::boards::create_column).get(routes::boards::list_columns),
        )
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
            axum::routing::patch(routes::boards::update_column)
                .delete(routes::boards::delete_column),
        )
        // Tasks
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/tasks",
            axum::routing::post(routes::tasks::create_task).get(routes::tasks::list_tasks),
        )
        // Board presence (heartbeat / leave)
        .route(
            "/api/workspaces/{ws}/boards/{board_id}/presence",
            axum::routing::post(routes::presence::heartbeat).delete(routes::presence::leave),
        )
        // Document presence (heartbeat / leave)
        .route(
            "/api/workspaces/{ws}/documents/{slug}/presence",
            axum::routing::post(routes::presence::document_heartbeat)
                .delete(routes::presence::document_leave),
        )
        .route(
            "/api/workspaces/{ws}/tasks",
            axum::routing::get(routes::tasks::list_workspace_tasks),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}",
            axum::routing::get(routes::tasks::get_task)
                .patch(routes::tasks::update_task)
                .delete(routes::tasks::delete_task),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/move",
            axum::routing::post(routes::tasks::move_task),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/assignees",
            axum::routing::get(routes::tasks::list_assignees).post(routes::tasks::add_assignee),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
            axum::routing::delete(routes::tasks::remove_assignee),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/references",
            axum::routing::get(routes::tasks::list_references)
                .post(routes::tasks::create_reference),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
            axum::routing::delete(routes::tasks::delete_reference),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/attachments",
            axum::routing::post(routes::tasks::upload_attachment)
                .get(routes::tasks::list_attachments)
                .layer(axum::extract::DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content",
            get(routes::tasks::download_attachment),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
            axum::routing::patch(routes::tasks::rename_attachment)
                .delete(routes::tasks::delete_attachment),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
            axum::routing::post(routes::tasks::upload_comment_attachment)
                .get(routes::tasks::list_comment_attachments)
                .layer(axum::extract::DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
            axum::routing::post(routes::tasks::create_comment_draft),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
            axum::routing::delete(routes::tasks::cancel_comment_draft),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
            axum::routing::post(routes::tasks::upload_comment_draft_attachment)
                .layer(axum::extract::DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
            get(routes::tasks::download_comment_attachment),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
            axum::routing::delete(routes::tasks::delete_comment_attachment),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/backlinks",
            axum::routing::get(routes::tasks::list_backlinks),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/checklist",
            axum::routing::get(routes::tasks::list_checklist)
                .post(routes::tasks::create_checklist_item),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
            axum::routing::patch(routes::tasks::update_checklist_item)
                .delete(routes::tasks::delete_checklist_item),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
            axum::routing::post(routes::tasks::promote_checklist_item),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/subtasks",
            axum::routing::get(routes::tasks::list_subtasks).post(routes::tasks::create_subtask),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/promote",
            axum::routing::post(routes::tasks::promote_subtask),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/activity",
            axum::routing::get(routes::tasks::list_activity),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comments",
            axum::routing::get(routes::tasks::list_comments).post(routes::tasks::create_comment),
        )
        .route(
            "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
            axum::routing::patch(routes::tasks::update_comment)
                .delete(routes::tasks::delete_comment),
        )
        .route(
            "/api/workspaces/{ws}/activity",
            axum::routing::get(routes::tasks::list_workspace_activity),
        )
        // Workspace security audit log (owner/admin only)
        .route(
            "/api/workspaces/{ws}/audit",
            axum::routing::get(routes::audit::list_workspace_audit),
        )
        // Documents
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}/documents",
            axum::routing::post(routes::documents::create_document)
                .get(routes::documents::list_documents),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}",
            get(routes::documents::get_document)
                .patch(routes::documents::update_document)
                .delete(routes::documents::delete_document),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/content",
            axum::routing::put(routes::documents::update_content),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/history",
            get(routes::documents::list_history),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/revisions/{seq}",
            get(routes::documents::get_revision_content),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/backlinks",
            get(routes::documents::list_backlinks),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/frontmatter",
            get(routes::documents::get_frontmatter),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/attachments",
            axum::routing::post(routes::documents::upload_attachment)
                .get(routes::documents::list_attachments),
        )
        .route(
            "/api/workspaces/{ws}/attachments/{attachment_id}",
            get(routes::documents::download_attachment)
                .delete(routes::documents::delete_attachment),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
            axum::routing::post(routes::documents::upload_comment_attachment)
                .get(routes::documents::list_comment_attachments)
                .layer(axum::extract::DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comment-drafts",
            axum::routing::post(routes::documents::create_comment_draft),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}",
            axum::routing::delete(routes::documents::cancel_comment_draft),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
            axum::routing::post(routes::documents::upload_comment_draft_attachment)
                .layer(axum::extract::DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
            get(routes::documents::download_comment_attachment)
                .delete(routes::documents::delete_comment_attachment),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/move",
            axum::routing::patch(routes::documents::move_document),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/copy",
            axum::routing::post(routes::documents::copy_document),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comments",
            get(routes::documents::list_comments).post(routes::documents::create_comment),
        )
        .route(
            "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
            axum::routing::patch(routes::documents::update_comment)
                .delete(routes::documents::delete_comment),
        )
        // Folders
        .route(
            "/api/workspaces/{ws}/projects/{project_slug}/folders",
            axum::routing::post(routes::folders::create_folder).get(routes::folders::list_folders),
        )
        .route(
            "/api/workspaces/{ws}/folders/{folder_id}",
            get(routes::folders::get_folder)
                .patch(routes::folders::rename_folder)
                .delete(routes::folders::delete_folder),
        )
        .route(
            "/api/workspaces/{ws}/folders/{folder_id}/move",
            axum::routing::patch(routes::folders::move_folder),
        )
        .route(
            "/api/workspaces/{ws}/folders/{folder_id}/copy",
            axum::routing::post(routes::folders::copy_folder),
        )
        // Webhooks (admin-only subscription CRUD + delivery log)
        .route(
            "/api/workspaces/{ws}/webhooks",
            axum::routing::post(routes::webhooks::create_webhook)
                .get(routes::webhooks::list_webhooks),
        )
        .route(
            "/api/workspaces/{ws}/webhooks/{webhook_id}",
            get(routes::webhooks::get_webhook)
                .patch(routes::webhooks::update_webhook)
                .delete(routes::webhooks::delete_webhook),
        )
        .route(
            "/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries",
            get(routes::webhooks::list_webhook_deliveries),
        )
        // Integration configs (admin-only)
        .route(
            "/api/workspaces/{ws}/integration-configs",
            axum::routing::post(routes::integration_configs::create_integration_config)
                .get(routes::integration_configs::list_integration_configs),
        )
        .route(
            "/api/workspaces/{ws}/integration-configs/{config_id}",
            get(routes::integration_configs::get_integration_config)
                .patch(routes::integration_configs::patch_integration_config)
                .delete(routes::integration_configs::delete_integration_config),
        )
        // Automation rules (admin-only)
        .route(
            "/api/workspaces/{ws}/automation-rules",
            axum::routing::post(routes::automation_rules::create_automation_rule)
                .get(routes::automation_rules::list_automation_rules),
        )
        .route(
            "/api/workspaces/{ws}/automation-rules/{rule_id}",
            get(routes::automation_rules::get_automation_rule)
                .patch(routes::automation_rules::patch_automation_rule)
                .delete(routes::automation_rules::delete_automation_rule),
        )
        // Live updates (Server-Sent Events)
        .route(
            "/api/workspaces/{ws}/events",
            get(routes::events::stream_events),
        )
        // Search
        .route("/api/workspaces/{ws}/search", get(routes::search::search))
        .route(
            "/api/workspaces/{ws}/semantic-search",
            get(routes::semantic_search::semantic_search),
        )
        .layer(axum_middleware::from_fn(
            crate::auth::csrf::require_csrf_for_cookie_mutations,
        ))
        // Runs after `require_authn` (layers execute outermost-first, so this is
        // applied before the authn layer in source order): the `Principal` is
        // present in extensions by the time the limiter reads it.
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::rate_limit::require_rate_limit,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::require_authn,
        ))
        .with_state(state.clone());

    // burst_size(5) and per_second(1) are non-zero, so finish() always returns Some here.
    #[allow(clippy::expect_used)]
    let activate_config = {
        let mut b = GovernorConfigBuilder::default();
        let cfg = b
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("governor config");
        std::sync::Arc::new(cfg)
    };

    // Per-IP governor for the public, unauthenticated integration-ingest route.
    // Same construction as the login/activate limiters; the quota is a little
    // higher because a single GitHub source IP fans out deliveries for many
    // repos/workspaces, and a rejected delivery is retried by GitHub.
    // burst_size and per_second are non-zero, so finish() always returns Some here.
    #[allow(clippy::expect_used)]
    let ingest_config = {
        let mut b = GovernorConfigBuilder::default();
        let cfg = b
            .per_second(5)
            .burst_size(20)
            .finish()
            .expect("governor config");
        std::sync::Arc::new(cfg)
    };

    let public = Router::new()
        .route("/health", get(routes::health::health))
        .route("/ready", get(routes::health::ready))
        .route("/version", get(routes::health::version))
        .route(
            "/api/auth/login",
            axum::routing::post(routes::auth::login).layer(GovernorLayer::new(login_config)),
        )
        .route(
            "/api/activate/{token}",
            get(routes::activate::get_activation_info)
                .post(routes::activate::post_activate)
                .layer(GovernorLayer::new(activate_config)),
        )
        // External event ingestion (public; HMAC-verified by the extractor,
        // per-IP rate-limited to bound abuse of this unauthenticated route)
        .route(
            "/api/workspaces/{ws}/integrations/{integration}/events",
            axum::routing::post(routes::integrations_ingest::ingest_github_event)
                .layer(GovernorLayer::new(ingest_config)),
        )
        .route("/openapi.json", get(routes::openapi::openapi_json))
        .merge(routes::openapi::scalar_router())
        .with_state(state.clone());

    let router = public.merge(protected);
    apply_layers(router)
}

/// Wraps `router` with the standard request-id / trace / problem-stamp layer stack.
///
/// The trace layer opens one span per request carrying the method, URI, and the
/// `x-request-id` set by the outer request-id layer, so every log emitted while
/// handling a request is correlated by that id. Request start, completion (with
/// status and latency), and failures are logged at INFO/ERROR.
///
/// `/health`, `/ready`, and `/version` are intentionally excluded: they are polled
/// at high frequency by probes and carry no useful per-request signal. Their span
/// is disabled, and the lifecycle callbacks short-circuit on a disabled span so
/// nothing is logged for them (a failing readiness probe still logs from its own
/// handler).
fn apply_layers(router: Router) -> Router {
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|request: &axum::http::Request<_>| {
            if matches!(request.uri().path(), "/health" | "/version" | "/ready") {
                return tracing::Span::none();
            }

            let request_id = request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-");

            tracing::info_span!(
                "http",
                method = %request.method(),
                uri = %request.uri(),
                request_id = %request_id,
            )
        })
        .on_request(|_request: &axum::http::Request<_>, span: &tracing::Span| {
            if span.is_disabled() {
                return;
            }
            tracing::info!("started processing request");
        })
        .on_response(
            |response: &axum::http::Response<_>,
             latency: std::time::Duration,
             span: &tracing::Span| {
                if span.is_disabled() {
                    return;
                }
                tracing::info!(
                    status = response.status().as_u16(),
                    latency = ?latency,
                    "finished processing request"
                );
            },
        )
        .on_failure(
            |error: ServerErrorsFailureClass,
             latency: std::time::Duration,
             span: &tracing::Span| {
                if span.is_disabled() {
                    return;
                }
                tracing::error!(error = %error, latency = ?latency, "request failed");
            },
        );

    router
        .layer(axum_middleware::from_fn(
            crate::middleware::problem_stamp::problem_stamp,
        ))
        .layer(trace_layer)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

/// Test helper: builds a minimal app with a single route and the full middleware stack.
///
/// Used by `tests/error_model.rs` to exercise the problem-stamp middleware without
/// starting a real server.
pub fn test_app_with_route(path: &str, handler: axum::routing::MethodRouter) -> Router {
    apply_layers(Router::new().route(path, handler))
}
