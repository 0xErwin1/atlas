#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::{Router, middleware as axum_middleware, routing::get};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod auth;
pub mod authz;
pub mod config;
pub mod error;
pub mod middleware;
pub mod persistence;
pub mod routes;
pub mod services;
pub mod state;

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

    let protected = Router::new()
        .route("/v1/auth/logout", axum::routing::post(routes::auth::logout))
        .route("/v1/auth/me", get(routes::auth::me))
        .route(
            "/v1/auth/change-password",
            axum::routing::post(routes::auth::change_password),
        )
        // Self-service profile (any authenticated user)
        .route(
            "/v1/users/me",
            axum::routing::patch(routes::auth::update_me),
        )
        // Self-service UI state (human users only; agents are rejected at the handler)
        .route(
            "/v1/me/ui-state",
            get(routes::ui_state::get_ui_state).put(routes::ui_state::set_ui_state),
        )
        // Server metadata (any authenticated principal)
        .route("/v1/meta", get(routes::health::meta))
        // Users (root-only)
        .route(
            "/v1/users",
            axum::routing::post(routes::users::create_user).get(routes::users::list_users),
        )
        .route(
            "/v1/users/{user_id}/disable",
            axum::routing::post(routes::users::disable_user),
        )
        .route(
            "/v1/users/{user_id}/enable",
            axum::routing::post(routes::users::enable_user),
        )
        .route(
            "/v1/users/{user_id}/reset-password",
            axum::routing::post(routes::users::reset_password),
        )
        // Workspace
        .route(
            "/v1/workspaces",
            get(routes::workspaces::list_workspaces).post(routes::workspaces::create_workspace),
        )
        .route(
            "/v1/workspaces/{ws}",
            get(routes::workspaces::get_workspace),
        )
        // API keys
        .route(
            "/v1/workspaces/{ws}/api-keys",
            axum::routing::post(routes::api_keys::create_api_key)
                .get(routes::api_keys::list_api_keys),
        )
        .route(
            "/v1/workspaces/{ws}/api-keys/{key_id}/revoke",
            axum::routing::post(routes::api_keys::revoke_api_key),
        )
        // Projects
        .route(
            "/v1/workspaces/{ws}/projects",
            axum::routing::post(routes::projects::create_project)
                .get(routes::projects::list_projects),
        )
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}",
            get(routes::projects::get_project)
                .patch(routes::projects::update_project)
                .delete(routes::projects::delete_project),
        )
        // Project grants
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}/grants",
            axum::routing::post(routes::grants::create_project_grant)
                .get(routes::grants::list_project_grants),
        )
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
            axum::routing::delete(routes::grants::delete_project_grant),
        )
        // Workspace grants
        .route(
            "/v1/workspaces/{ws}/grants",
            axum::routing::post(routes::grants::create_workspace_grant)
                .get(routes::grants::list_workspace_grants),
        )
        .route(
            "/v1/workspaces/{ws}/grants/{grant_id}",
            axum::routing::delete(routes::grants::delete_workspace_grant),
        )
        // Workspace members (principals addressable by a grant)
        .route(
            "/v1/workspaces/{ws}/members",
            get(routes::members::list_workspace_members),
        )
        // Boards
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}/boards",
            axum::routing::post(routes::boards::create_board).get(routes::boards::list_boards),
        )
        .route(
            "/v1/workspaces/{ws}/boards/{board_id}",
            axum::routing::get(routes::boards::get_board)
                .patch(routes::boards::update_board)
                .delete(routes::boards::delete_board),
        )
        .route(
            "/v1/workspaces/{ws}/boards/{board_id}/columns",
            axum::routing::post(routes::boards::create_column).get(routes::boards::list_columns),
        )
        .route(
            "/v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
            axum::routing::patch(routes::boards::update_column)
                .delete(routes::boards::delete_column),
        )
        // Tasks
        .route(
            "/v1/workspaces/{ws}/boards/{board_id}/tasks",
            axum::routing::post(routes::tasks::create_task).get(routes::tasks::list_tasks),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}",
            axum::routing::get(routes::tasks::get_task)
                .patch(routes::tasks::update_task)
                .delete(routes::tasks::delete_task),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/move",
            axum::routing::post(routes::tasks::move_task),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/assignees",
            axum::routing::get(routes::tasks::list_assignees).post(routes::tasks::add_assignee),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
            axum::routing::delete(routes::tasks::remove_assignee),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/references",
            axum::routing::get(routes::tasks::list_references)
                .post(routes::tasks::create_reference),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
            axum::routing::delete(routes::tasks::delete_reference),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/backlinks",
            axum::routing::get(routes::tasks::list_backlinks),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/checklist",
            axum::routing::get(routes::tasks::list_checklist)
                .post(routes::tasks::create_checklist_item),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
            axum::routing::patch(routes::tasks::update_checklist_item)
                .delete(routes::tasks::delete_checklist_item),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
            axum::routing::post(routes::tasks::promote_checklist_item),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/subtasks",
            axum::routing::get(routes::tasks::list_subtasks).post(routes::tasks::create_subtask),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/promote",
            axum::routing::post(routes::tasks::promote_subtask),
        )
        .route(
            "/v1/workspaces/{ws}/tasks/{readable_id}/activity",
            axum::routing::get(routes::tasks::list_activity),
        )
        // Documents
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}/documents",
            axum::routing::post(routes::documents::create_document)
                .get(routes::documents::list_documents),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}",
            get(routes::documents::get_document)
                .patch(routes::documents::update_document)
                .delete(routes::documents::delete_document),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/content",
            axum::routing::put(routes::documents::update_content),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/history",
            get(routes::documents::list_history),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/revisions/{seq}",
            get(routes::documents::get_revision_content),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/backlinks",
            get(routes::documents::list_backlinks),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/frontmatter",
            get(routes::documents::get_frontmatter),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/attachments",
            axum::routing::post(routes::documents::upload_attachment)
                .get(routes::documents::list_attachments),
        )
        .route(
            "/v1/workspaces/{ws}/attachments/{attachment_id}",
            get(routes::documents::download_attachment)
                .delete(routes::documents::delete_attachment),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/move",
            axum::routing::patch(routes::documents::move_document),
        )
        .route(
            "/v1/workspaces/{ws}/documents/{slug}/copy",
            axum::routing::post(routes::documents::copy_document),
        )
        // Folders
        .route(
            "/v1/workspaces/{ws}/projects/{project_slug}/folders",
            axum::routing::post(routes::folders::create_folder).get(routes::folders::list_folders),
        )
        .route(
            "/v1/workspaces/{ws}/folders/{folder_id}",
            get(routes::folders::get_folder)
                .patch(routes::folders::rename_folder)
                .delete(routes::folders::delete_folder),
        )
        .route(
            "/v1/workspaces/{ws}/folders/{folder_id}/move",
            axum::routing::patch(routes::folders::move_folder),
        )
        .route(
            "/v1/workspaces/{ws}/folders/{folder_id}/copy",
            axum::routing::post(routes::folders::copy_folder),
        )
        // Search
        .route("/v1/workspaces/{ws}/search", get(routes::search::search))
        .layer(axum_middleware::from_fn(
            crate::auth::csrf::require_csrf_for_cookie_mutations,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::require_authn,
        ))
        .with_state(state.clone());

    let public = Router::new()
        .route("/health", get(routes::health::health))
        .route("/version", get(routes::health::version))
        .route(
            "/v1/auth/login",
            axum::routing::post(routes::auth::login).layer(GovernorLayer::new(login_config)),
        )
        .route("/openapi.json", get(routes::openapi::openapi_json))
        .merge(routes::openapi::scalar_router())
        .with_state(state.clone());

    let router = public.merge(protected);
    apply_layers(router)
}

/// Wraps `router` with the standard request-id / trace / problem-stamp layer stack.
fn apply_layers(router: Router) -> Router {
    router
        .layer(axum_middleware::from_fn(
            crate::middleware::problem_stamp::problem_stamp,
        ))
        .layer(TraceLayer::new_for_http())
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
