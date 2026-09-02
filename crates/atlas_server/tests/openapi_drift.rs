#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_server::routes::openapi::openapi;
use serde_json::Value;

/// All schema component names that must be present in the generated OpenAPI document.
///
/// If a new DTO is added to the `ApiDoc` components list, it must appear here too —
/// and vice versa. This test is the guard against silent drift.
const EXPECTED_SCHEMAS: &[&str] = &[
    "LoginRequest",
    "LoginResponse",
    "MeResponse",
    "AgentIdentityDto",
    "ServerMetaDto",
    "UiStateDto",
    "UpdateUiStateRequest",
    "ChangePasswordRequest",
    "UpdateMeRequest",
    "ResetPasswordRequest",
    "CreateUserRequest",
    "UserDto",
    "ApiKeyCreated",
    "ApiKeyDto",
    "ApiKeyScope",
    "UpdateApiKeyRequest",
    "CreateProjectRequest",
    "UpdateProjectRequest",
    "ProjectDto",
    "CreateGrantRequest",
    "GrantPrincipal",
    "GrantDto",
    "PrincipalDto",
    "CreateWorkspaceRequest",
    "UpdateWorkspaceRequest",
    "AdminUpdateWorkspaceRequest",
    "WorkspaceDto",
    "ProblemDetails",
    "CreateDocumentRequest",
    "UpdateDocumentRequest",
    "UpdateContentRequest",
    "DocumentBacklinkSourceDto",
    "DocumentContentEditRequest",
    "DocumentLineEditRequest",
    "MoveDocumentRequest",
    "DocumentMoveBatchRequest",
    "DocumentMoveBatchItemRequest",
    "DocumentMoveBatchResultDto",
    "CopyDocumentRequest",
    "DocumentDto",
    "DocumentCompactDto",
    "DocumentContentRangeQuery",
    "DocumentContentRangeDto",
    "DocumentLineDto",
    "DocumentContentSearchRequest",
    "DocumentContentSearchDto",
    "DocumentSearchMatchDto",
    "DocumentSearchMode",
    "DocumentSummaryDto",
    "RevisionMetaDto",
    "RevisionContentDto",
    "BacklinkDto",
    "FrontmatterDto",
    "CommentBacklinkSourceDto",
    "CommentBacklinkParentDto",
    "AttachmentDto",
    "AttachmentOwnerDto",
    "WorkspaceAttachmentDto",
    "RenameAttachmentRequest",
    "CommentAttachmentDto",
    "CommentDraftDto",
    "ActorDto",
    "ConflictProblemDto",
    "BoardDto",
    "BoardSummaryDto",
    "BoardPresenceResponse",
    "DocumentPresenceResponse",
    "ColumnDto",
    "CreateBoardRequest",
    "UpdateBoardRequest",
    "MoveBoardRequest",
    "CreateColumnRequest",
    "UpdateColumnRequest",
    "TaskDto",
    "TaskSummaryDto",
    "TaskPropertiesDto",
    "CreateTaskRequest",
    "CreateTaskResponseDto",
    "UpdateTaskRequest",
    "MoveTaskRequest",
    "AssigneeDto",
    "AddAssigneeRequest",
    "ReferenceDto",
    "ReferenceOriginDto",
    "UnifiedReferenceDto",
    "TaskAttachmentDto",
    "RenameTaskAttachmentRequest",
    "TaskBacklinkDto",
    "TaskGraphDto",
    "TaskGraphNodeDto",
    "TaskGraphEdgeDto",
    "CreateReferenceRequest",
    "CreateReferenceBatchRequest",
    "CreateReferenceBatchResultDto",
    "ChecklistItemDto",
    "CreateChecklistItemRequest",
    "CreateSubtaskRequest",
    "SetTaskParentRequest",
    "UpdateChecklistItemRequest",
    "PromotionDto",
    "PromoteChecklistItemRequest",
    "ActivityEntryDto",
    "CommentDto",
    "CommentLinkProjectionDto",
    "CommentLinkTargetDto",
    "CommentListResponseDto",
    "Page_CommentFeedEntryDto",
    "CreateCommentRequest",
    "UpdateCommentRequest",
    "Page_CommentDto",
    "AuditEntryDto",
    "SearchHitDto",
    "SearchKindDto",
    "SemanticSearchHitDto",
    "SemanticSearchKindDto",
    "SemanticSearchSourceDto",
    "SemanticReindexPlanDto",
    "SemanticReindexStartedDto",
    "CreateFolderRequest",
    "RenameFolderRequest",
    "MoveFolderRequest",
    "CopyFolderRequest",
    "FolderDto",
    "TagDto",
    "CreateTagRequest",
    "UpdateTagRequest",
    "StatusTemplateDto",
    "PlatformStatusTemplateDto",
    "CreateStatusTemplateRequest",
    "UpdateStatusTemplateRequest",
    "PropertyDefinitionDto",
    "CreatePropertyDefinitionRequest",
    "SavedSearchDto",
    "CreateSavedSearchRequest",
    "RenameSavedSearchRequest",
    "TaskViewDto",
    "TaskViewFiltersDto",
    "CreateTaskViewRequest",
    "UpdateTaskViewRequest",
    "Page_FolderDto",
    "Page_GrantDto",
    "Page_DocumentSummaryDto",
    "Page_BacklinkDto",
    "Page_TaskBacklinkDto",
    "Page_ProjectDto",
    "Page_TaskSummaryDto",
    "Page_ActivityEntryDto",
    "Page_AuditEntryDto",
    "Page_BoardSummaryDto",
    "Page_ApiKeyDto",
    "CreateUserApiKeyRequest",
    "InitialGrantRequest",
    "SetSystemAdminRequest",
    "UserMembershipDto",
    "ApiKeyGrantDto",
    "GrantedByDto",
    "UpdateMemberRoleRequest",
    "AddMemberRequest",
    "CreateUserResponse",
    "ActivationLinkResponse",
    "ActivationInfoDto",
    "ActivatePasswordRequest",
    "CreateGroupRequest",
    "GroupDto",
    "GroupMemberDto",
    "AddGroupMemberRequest",
    "WebhookDto",
    "WebhookCreatedDto",
    "CreateWebhookRequest",
    "UpdateWebhookRequest",
    "WebhookDeliveryDto",
    "Page_WebhookDto",
    "Page_WebhookDeliveryDto",
    "CreateIntegrationConfigRequest",
    "UpdateIntegrationConfigRequest",
    "IntegrationConfigDto",
    "IntegrationConfigCreatedDto",
    "CreateAutomationRuleRequest",
    "PatchAutomationRuleRequest",
    "AutomationRuleDto",
    "Page_AutomationRuleDto",
    "TrashKindDto",
    "TrashItemDto",
    "RestoreTrashItemRequest",
    "Page_TrashItemDto",
    "PurgeStatusDto",
    "PurgeTrashItemRequest",
    "PurgeStatusDtoResponse",
];

/// Every unique OpenAPI path this server declares, extracted from the old
/// hand-maintained route registry (`v2-e3-s2` PR5 deleted it; this literal
/// list is its openapi_path values, frozen, since `RouteDeclaration` carries no
/// OpenAPI-path field and deriving the OpenAPI document from the REG-5
/// registry is out of scope for this slice, deferred to S4 — see the spec's
/// Non-Goals). This test's drift coverage is unchanged: it still fails when
/// a route's OpenAPI annotation and this list disagree.
const EXPECTED_OPENAPI_PATHS: &[&str] = &[
    "/api/activate/{token}",
    "/api/admin/audit",
    "/api/admin/status-templates",
    "/api/admin/status-templates/{template_id}",
    "/api/admin/trash",
    "/api/admin/trash/purge",
    "/api/admin/trash/purges/{operation_id}",
    "/api/admin/trash/restore",
    "/api/admin/workspaces",
    "/api/admin/workspaces/{ws}",
    "/api/api-keys",
    "/api/api-keys/{key_id}",
    "/api/api-keys/{key_id}/grants",
    "/api/api-keys/{key_id}/grants/{grant_id}",
    "/api/auth/change-password",
    "/api/auth/login",
    "/api/auth/logout",
    "/api/auth/me",
    "/api/meta",
    "/api/me/ui-state",
    "/api/users",
    "/api/users/me",
    "/api/users/{user_id}/activation-link",
    "/api/users/{user_id}/disable",
    "/api/users/{user_id}/enable",
    "/api/users/{user_id}/memberships",
    "/api/users/{user_id}/reset-password",
    "/api/users/{user_id}/system-admin",
    "/api/workspaces",
    "/api/workspaces/{ws}",
    "/api/workspaces/{ws}/activity",
    "/api/workspaces/{ws}/assignable-users",
    "/api/workspaces/{ws}/attachments",
    "/api/workspaces/{ws}/attachments/{attachment_id}",
    "/api/workspaces/{ws}/audit",
    "/api/workspaces/{ws}/automation-rules",
    "/api/workspaces/{ws}/automation-rules/{rule_id}",
    "/api/workspaces/{ws}/boards/{board_id}",
    "/api/workspaces/{ws}/boards/{board_id}/apply-status-templates",
    "/api/workspaces/{ws}/boards/{board_id}/archive",
    "/api/workspaces/{ws}/boards/{board_id}/columns",
    "/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
    "/api/workspaces/{ws}/boards/{board_id}/move",
    "/api/workspaces/{ws}/boards/{board_id}/presence",
    "/api/workspaces/{ws}/boards/{board_id}/tasks",
    "/api/workspaces/{ws}/boards/{board_id}/unarchive",
    "/api/workspaces/{ws}/documents/moves/batch",
    "/api/workspaces/{ws}/documents/{slug}",
    "/api/workspaces/{ws}/documents/{slug}/attachments",
    "/api/workspaces/{ws}/documents/{slug}/backlinks",
    "/api/workspaces/{ws}/documents/{slug}/comment-drafts",
    "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}",
    "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
    "/api/workspaces/{ws}/documents/{slug}/comments",
    "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
    "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
    "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
    "/api/workspaces/{ws}/documents/{slug}/compact",
    "/api/workspaces/{ws}/documents/{slug}/content",
    "/api/workspaces/{ws}/documents/{slug}/content/range",
    "/api/workspaces/{ws}/documents/{slug}/content/search",
    "/api/workspaces/{ws}/documents/{slug}/copy",
    "/api/workspaces/{ws}/documents/{slug}/frontmatter",
    "/api/workspaces/{ws}/documents/{slug}/history",
    "/api/workspaces/{ws}/documents/{slug}/move",
    "/api/workspaces/{ws}/documents/{slug}/presence",
    "/api/workspaces/{ws}/documents/{slug}/revisions/{seq}",
    "/api/workspaces/{ws}/folders/{folder_id}",
    "/api/workspaces/{ws}/folders/{folder_id}/copy",
    "/api/workspaces/{ws}/folders/{folder_id}/move",
    "/api/workspaces/{ws}/grants",
    "/api/workspaces/{ws}/grants/{grant_id}",
    "/api/workspaces/{ws}/groups",
    "/api/workspaces/{ws}/groups/{group_id}",
    "/api/workspaces/{ws}/groups/{group_id}/members",
    "/api/workspaces/{ws}/groups/{group_id}/members/{user_id}",
    "/api/workspaces/{ws}/integration-configs",
    "/api/workspaces/{ws}/integration-configs/{config_id}",
    "/api/workspaces/{ws}/integrations/{integration}/events",
    "/api/workspaces/{ws}/members",
    "/api/workspaces/{ws}/members/{user_id}",
    "/api/workspaces/{ws}/projects",
    "/api/workspaces/{ws}/projects/{project_slug}",
    "/api/workspaces/{ws}/projects/{project_slug}/boards",
    "/api/workspaces/{ws}/projects/{project_slug}/documents",
    "/api/workspaces/{ws}/projects/{project_slug}/folders",
    "/api/workspaces/{ws}/projects/{project_slug}/grants",
    "/api/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
    "/api/workspaces/{ws}/property-definitions",
    "/api/workspaces/{ws}/property-definitions/{property_definition_id}",
    "/api/workspaces/{ws}/saved-searches",
    "/api/workspaces/{ws}/saved-searches/{id}",
    "/api/workspaces/{ws}/search",
    "/api/workspaces/{ws}/semantic-search",
    "/api/workspaces/{ws}/semantic-search/reindex",
    "/api/workspaces/{ws}/status-templates",
    "/api/workspaces/{ws}/status-templates/{template_id}",
    "/api/workspaces/{ws}/tags",
    "/api/workspaces/{ws}/tags/{tag_id}",
    "/api/workspaces/{ws}/tags/used",
    "/api/workspaces/{ws}/tasks",
    "/api/workspaces/{ws}/tasks/{readable_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/activity",
    "/api/workspaces/{ws}/tasks/{readable_id}/assignees",
    "/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
    "/api/workspaces/{ws}/tasks/{readable_id}/attachments",
    "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content",
    "/api/workspaces/{ws}/tasks/{readable_id}/backlinks",
    "/api/workspaces/{ws}/tasks/{readable_id}/checklist",
    "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
    "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
    "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
    "/api/workspaces/{ws}/tasks/{readable_id}/comments",
    "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
    "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
    "/api/workspaces/{ws}/tasks/{readable_id}/graph",
    "/api/workspaces/{ws}/tasks/{readable_id}/move",
    "/api/workspaces/{ws}/tasks/{readable_id}/parent",
    "/api/workspaces/{ws}/tasks/{readable_id}/promote",
    "/api/workspaces/{ws}/tasks/{readable_id}/references",
    "/api/workspaces/{ws}/tasks/{readable_id}/references/batch",
    "/api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
    "/api/workspaces/{ws}/tasks/{readable_id}/subtasks",
    "/api/workspaces/{ws}/task-views",
    "/api/workspaces/{ws}/task-views/{id}",
    "/api/workspaces/{ws}/webhooks",
    "/api/workspaces/{ws}/webhooks/{webhook_id}",
    "/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries",
    "/health",
    "/ready",
    "/version",
];

#[test]
fn openapi_document_contains_required_schemas() {
    let doc = openapi();

    let components = doc
        .components
        .as_ref()
        .expect("OpenAPI document must have a components section");

    let schemas = &components.schemas;

    for name in EXPECTED_SCHEMAS {
        assert!(
            schemas.contains_key(*name),
            "expected schema '{name}' is missing from OpenAPI components"
        );
    }

    assert_eq!(
        schemas.len(),
        EXPECTED_SCHEMAS.len(),
        "OpenAPI component schema count mismatch: expected {}, got {}. \
         Update EXPECTED_SCHEMAS in openapi_drift.rs when adding or removing DTOs.",
        EXPECTED_SCHEMAS.len(),
        schemas.len()
    );
}

/// Every path in `EXPECTED_OPENAPI_PATHS` must appear in the OpenAPI
/// document, and the set of unique paths in that list must exactly match the
/// set of paths in the generated document.
///
/// This test's data source used to be the old hand-maintained route registry,
/// retired in `v2-e3-s2` PR5; see `EXPECTED_OPENAPI_PATHS`'s doc for why it
/// is now a frozen literal instead. The reverse direction (a route or
/// annotation added without an `EXPECTED_OPENAPI_PATHS` entry) is not
/// automatically caught — this list is what developers must update when
/// adding or removing routes with an OpenAPI-documented path.
#[test]
fn openapi_document_paths_match_router() {
    let doc = openapi();
    let doc_paths = &doc.paths.paths;

    let expected: std::collections::BTreeSet<&'static str> =
        EXPECTED_OPENAPI_PATHS.iter().copied().collect();
    assert_eq!(
        expected.len(),
        EXPECTED_OPENAPI_PATHS.len(),
        "EXPECTED_OPENAPI_PATHS must not contain duplicate paths"
    );

    for path in &expected {
        assert!(
            doc_paths.contains_key(*path),
            "route '{path}' is in EXPECTED_OPENAPI_PATHS but missing from the OpenAPI document; \
             add a #[utoipa::path] annotation and register it in ApiDoc paths()"
        );
    }

    assert_eq!(
        doc_paths.len(),
        expected.len(),
        "OpenAPI path count mismatch: expected {} unique paths, document has {}. \
         Update EXPECTED_OPENAPI_PATHS in openapi_drift.rs when adding or removing routes.",
        expected.len(),
        doc_paths.len()
    );
}

#[test]
fn openapi_document_has_correct_info() {
    let doc = openapi();

    assert_eq!(doc.info.title, "Atlas API");
    assert!(!doc.info.version.is_empty(), "version must not be empty");
}

#[test]
fn trash_operations_document_lifecycle_statuses_and_admin_scope() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");

    assert!(
        document
            .pointer("/tags")
            .and_then(Value::as_array)
            .is_some_and(|tags| tags.iter().any(|tag| tag["name"] == "trash")),
        "OpenAPI must describe the root/system-admin Trash tag"
    );

    assert_operation_statuses(&document, "/api/admin/trash", "get", &[200, 400, 401, 403]);
    let list = operation(&document, "/api/admin/trash", "get");
    let kind_parameter = list
        .get("parameters")
        .and_then(Value::as_array)
        .and_then(|parameters| {
            parameters
                .iter()
                .find(|parameter| parameter["name"] == "kind" && parameter["in"] == "query")
        });
    assert_eq!(
        kind_parameter.and_then(|parameter| parameter.pointer("/schema/$ref")),
        Some(&Value::String(
            "#/components/schemas/TrashKindDto".to_string()
        )),
        "the Trash kind filter must use the closed lifecycle enum"
    );
    assert_operation_statuses(
        &document,
        "/api/admin/trash/restore",
        "post",
        &[204, 401, 403, 404, 409],
    );
    assert_operation_statuses(
        &document,
        "/api/admin/trash/purge",
        "post",
        &[202, 204, 400, 401, 403, 404],
    );
    assert_operation_statuses(
        &document,
        "/api/admin/trash/purges/{operation_id}",
        "get",
        &[200, 401, 403, 404],
    );
}

#[test]
fn task_attachment_rename_operation_documents_typed_contract() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
    let path = "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}";

    assert_operation_statuses(&document, path, "patch", &[200, 401, 403, 404, 422]);
    let patch = operation(&document, path, "patch");
    assert_eq!(
        patch.pointer("/requestBody/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/RenameTaskAttachmentRequest".to_string()
        ))
    );
    assert_eq!(
        patch.pointer("/responses/200/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/TaskAttachmentDto".to_string()
        ))
    );
}

#[test]
fn compact_document_operation_documents_the_metadata_only_contract() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
    let compact = operation(
        &document,
        "/api/workspaces/{ws}/documents/{slug}/compact",
        "get",
    );

    assert_eq!(
        compact.pointer("/responses/200/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/DocumentCompactDto".into()
        ))
    );
    assert!(
        document
            .pointer("/components/schemas/DocumentCompactDto/properties/content")
            .is_none(),
        "the compact schema must not expose content"
    );
}

#[test]
fn reference_batch_operation_documents_its_bounded_result_contract() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
    let batch = operation(
        &document,
        "/api/workspaces/{ws}/tasks/{readable_id}/references/batch",
        "post",
    );

    assert_operation_statuses(
        &document,
        "/api/workspaces/{ws}/tasks/{readable_id}/references/batch",
        "post",
        &[200, 413, 422],
    );
    assert_eq!(
        batch.pointer("/requestBody/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/CreateReferenceBatchRequest".into()
        ))
    );
    assert!(
        batch
            .pointer("/responses/200/content/application~1json/schema/items/$ref")
            .is_some_and(|schema| schema == "#/components/schemas/CreateReferenceBatchResultDto"),
        "the batch success response must expose one typed result per input item"
    );
}

#[test]
fn document_move_batch_operation_documents_its_bounded_result_contract() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
    let path = "/api/workspaces/{ws}/documents/moves/batch";
    let batch = operation(&document, path, "post");

    assert_operation_statuses(&document, path, "post", &[200, 413, 422]);
    assert_eq!(
        batch.pointer("/requestBody/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/DocumentMoveBatchRequest".into()
        ))
    );
    assert!(
        batch
            .pointer("/responses/200/content/application~1json/schema/items/$ref")
            .is_some_and(|schema| schema == "#/components/schemas/DocumentMoveBatchResultDto"),
        "the batch success response must expose one typed result per input item"
    );
}

#[test]
fn comment_draft_attachment_operations_document_routes_statuses_and_binary_headers() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");

    for (parent_path, attachment_path) in [
        (
            "/api/workspaces/{ws}/tasks/{readable_id}",
            "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
        ),
        (
            "/api/workspaces/{ws}/documents/{slug}",
            "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        ),
    ] {
        let draft_path = format!("{parent_path}/comment-drafts");
        let upload_path = format!("{draft_path}/{{draft_id}}/attachments");
        let cancel_path = format!("{draft_path}/{{draft_id}}");

        assert_operation_statuses(
            &document,
            &draft_path,
            "post",
            &[200, 201, 404, 409, 410, 422],
        );
        assert_operation_statuses(
            &document,
            &upload_path,
            "post",
            &[200, 201, 404, 409, 410, 413, 422],
        );
        assert_operation_statuses(&document, &cancel_path, "delete", &[204, 404, 409, 410]);
        assert_operation_statuses(
            &document,
            &format!("{parent_path}/comments"),
            "post",
            &[200, 201, 404, 409, 410, 422],
        );
        assert_operation_statuses(
            &document,
            &format!("{parent_path}/comments/{{comment_id}}/attachments"),
            "get",
            &[200, 404, 410],
        );
        assert_operation_statuses(
            &document,
            &format!("{parent_path}/comments/{{comment_id}}/attachments/{{attachment_id}}"),
            "delete",
            &[204, 404, 410],
        );
        assert_operation_statuses(&document, attachment_path, "get", &[200, 404, 410]);

        assert_header_parameter(operation(&document, &draft_path, "post"), "x-create-token");
        assert_header_parameter(operation(&document, &upload_path, "post"), "x-upload-token");

        let create = operation(&document, &format!("{parent_path}/comments"), "post");
        assert!(
            create
                .pointer("/requestBody/content/application~1json/schema/$ref")
                .is_some_and(|schema| schema == "#/components/schemas/CreateCommentRequest"),
            "{parent_path} comment creation must use the shared CreateCommentRequest schema"
        );

        let get = operation(&document, attachment_path, "get");
        for header in [
            "Content-Type",
            "Content-Disposition",
            "X-Content-Type-Options",
        ] {
            assert!(
                get.pointer(&format!("/responses/200/headers/{header}"))
                    .is_some(),
                "{attachment_path} must document its {header} response header"
            );
        }
    }

    assert_eq!(
        document.pointer("/components/schemas/CommentDraftDto/type"),
        Some(&Value::String("object".into()))
    );

    for property in ["id", "expires_at"] {
        assert!(
            document
                .pointer(&format!(
                    "/components/schemas/CommentDraftDto/properties/{property}"
                ))
                .is_some(),
            "CommentDraftDto must expose {property}"
        );
    }

    for property in ["url", "markdown"] {
        assert!(
            document
                .pointer(&format!(
                    "/components/schemas/CommentAttachmentDto/properties/{property}"
                ))
                .is_some(),
            "CommentAttachmentDto must expose {property}"
        );
    }

    assert!(
        document
            .pointer("/components/schemas/CreateCommentRequest/properties/draft_id")
            .is_some(),
        "comment creation must expose the additive draft_id"
    );
}

fn assert_header_parameter(operation: &Value, name: &str) {
    assert!(
        operation
            .pointer("/parameters")
            .and_then(Value::as_array)
            .is_some_and(|parameters| parameters.iter().any(|parameter| {
                parameter.get("name") == Some(&Value::String(name.into()))
                    && parameter.get("in") == Some(&Value::String("header".into()))
                    && parameter.get("required") == Some(&Value::Bool(true))
            })),
        "operation must require the {name} header"
    );
}

fn assert_operation_statuses(document: &Value, path: &str, method: &str, statuses: &[u16]) {
    let operation = operation(document, path, method);

    for status in statuses {
        assert!(
            operation.pointer(&format!("/responses/{status}")).is_some(),
            "{method} {path} must document status {status}"
        );
    }
}

#[test]
fn full_comment_feed_query_is_documented_for_both_parent_routes() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");

    for path in [
        "/api/workspaces/{ws}/tasks/{readable_id}/comments",
        "/api/workspaces/{ws}/documents/{slug}/comments",
    ] {
        let pointer = format!("/paths/{}/get/parameters", path.replace('/', "~1"));
        let parameters = document.pointer(&pointer).and_then(Value::as_array);
        assert!(
            parameters.is_some_and(|parameters| parameters.iter().any(|parameter| {
                parameter.get("name") == Some(&Value::String("feed".into()))
                    && parameter.get("in") == Some(&Value::String("query".into()))
            })),
            "{path} must document the feed query selector"
        );
    }
}

#[test]
fn comment_freedom_contract_is_exact_for_feeds_backlinks_attachments_and_metadata() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");

    for path in [
        "/api/workspaces/{ws}/tasks/{readable_id}/comments",
        "/api/workspaces/{ws}/documents/{slug}/comments",
    ] {
        let get = operation(&document, path, "get");

        assert_eq!(
            get.pointer("/responses/200/content/application~1json/schema/$ref"),
            Some(&Value::String(
                "#/components/schemas/CommentListResponseDto".into()
            )),
            "{path} must preserve the compatible default comment page and opt-in full feed union"
        );
    }

    assert_eq!(
        document.pointer("/components/schemas/BacklinkDto/properties/comment_source/oneOf/1/$ref"),
        Some(&Value::String(
            "#/components/schemas/CommentBacklinkSourceDto".into()
        )),
        "backlinks must expose the authorized comment source projection"
    );

    assert_attachment_lifecycle(
        &document,
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
    );
    assert_attachment_lifecycle(
        &document,
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
    );

    let document_upload = operation(
        &document,
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        "post",
    );
    assert_eq!(
        document_upload.pointer("/requestBody/content/application~1octet-stream/schema/type"),
        Some(&Value::String("array".into())),
        "document comment uploads must accept raw binary request bytes"
    );
    assert_eq!(
        document_upload
            .pointer("/requestBody/content/application~1octet-stream/schema/items/format"),
        Some(&Value::String("int32".into())),
        "document comment uploads must identify each raw body byte"
    );
    assert!(
        document_upload
            .pointer("/parameters")
            .and_then(Value::as_array)
            .is_some_and(|parameters| parameters.iter().any(|parameter| {
                parameter.get("name") == Some(&Value::String("x-file-name".into()))
                    && parameter.get("in") == Some(&Value::String("header".into()))
                    && parameter.get("required") == Some(&Value::Bool(true))
            })),
        "document comment uploads must require the x-file-name header"
    );

    let limit = document
        .pointer("/components/schemas/ServerMetaDto/properties/max_attachment_bytes")
        .expect("server metadata must advertise the optional attachment limit");

    assert_eq!(
        limit.get("type"),
        Some(&serde_json::json!(["integer", "null"]))
    );
    assert_eq!(limit.get("format"), Some(&Value::String("int64".into())));
    assert_eq!(limit.get("minimum"), Some(&serde_json::json!(0)));

    let semantic_search_enabled = document
        .pointer("/components/schemas/ServerMetaDto/properties/semantic_search_enabled")
        .expect("server metadata must advertise semantic search availability");
    assert_eq!(
        semantic_search_enabled.get("type"),
        Some(&serde_json::json!(["boolean", "null"]))
    );
}

fn operation<'a>(document: &'a Value, path: &str, method: &str) -> &'a Value {
    let pointer = format!("/paths/{}/{}", path.replace('/', "~1"), method);

    document
        .pointer(&pointer)
        .unwrap_or_else(|| panic!("{method} {path} must be present in OpenAPI"))
}

fn assert_attachment_lifecycle(
    document: &Value,
    collection_path: &str,
    item_path: &str,
    content_path: &str,
) {
    let collection_get = operation(document, collection_path, "get");
    let collection_post = operation(document, collection_path, "post");

    assert_eq!(
        collection_get.pointer("/responses/200/content/application~1json/schema/items/$ref"),
        Some(&Value::String(
            "#/components/schemas/CommentAttachmentDto".into()
        )),
        "{collection_path} must list comment-owned attachment metadata"
    );
    assert_eq!(
        collection_post.pointer("/responses/201/content/application~1json/schema/$ref"),
        Some(&Value::String(
            "#/components/schemas/CommentAttachmentDto".into()
        )),
        "{collection_path} must upload comment-owned attachment metadata"
    );
    assert!(
        operation(document, item_path, "delete")
            .pointer("/responses/204")
            .is_some(),
        "{item_path} must delete a comment-owned attachment"
    );
    assert!(
        operation(document, content_path, "get")
            .pointer("/responses/200")
            .is_some(),
        "{content_path} must download comment-owned attachment content"
    );
}
