#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_server::routes::openapi::openapi;
use atlas_server::routes::registry::ROUTE_REGISTRY;
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
    "MoveDocumentRequest",
    "CopyDocumentRequest",
    "DocumentDto",
    "DocumentSummaryDto",
    "RevisionMetaDto",
    "RevisionContentDto",
    "BacklinkDto",
    "FrontmatterDto",
    "CommentBacklinkSourceDto",
    "CommentBacklinkParentDto",
    "AttachmentDto",
    "CommentAttachmentDto",
    "ActorDto",
    "ConflictProblemDto",
    "BoardDto",
    "BoardSummaryDto",
    "BoardPresenceResponse",
    "DocumentPresenceResponse",
    "ColumnDto",
    "CreateBoardRequest",
    "UpdateBoardRequest",
    "CreateColumnRequest",
    "UpdateColumnRequest",
    "TaskDto",
    "TaskSummaryDto",
    "TaskPropertiesDto",
    "CreateTaskRequest",
    "UpdateTaskRequest",
    "MoveTaskRequest",
    "AssigneeDto",
    "AddAssigneeRequest",
    "ReferenceDto",
    "ReferenceOriginDto",
    "UnifiedReferenceDto",
    "TaskAttachmentDto",
    "TaskBacklinkDto",
    "CreateReferenceRequest",
    "ChecklistItemDto",
    "CreateChecklistItemRequest",
    "CreateSubtaskRequest",
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
    "CreateFolderRequest",
    "RenameFolderRequest",
    "MoveFolderRequest",
    "CopyFolderRequest",
    "FolderDto",
    "TagDto",
    "CreateTagRequest",
    "UpdateTagRequest",
    "StatusTemplateDto",
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

/// Every route that declares an `openapi_path` in ROUTE_REGISTRY must appear in
/// the OpenAPI document. The set of unique OpenAPI paths in ROUTE_REGISTRY must
/// exactly match the set of paths in the generated document.
///
/// Coverage: ROUTE_REGISTRY → router is enforced by `all_registry_entries_are_wired_in_router`.
/// ROUTE_REGISTRY → OpenAPI doc is enforced here. The reverse direction (a route or
/// annotation added without a ROUTE_REGISTRY entry) is not automatically caught —
/// ROUTE_REGISTRY is the authoritative list developers must update when adding routes.
#[test]
fn openapi_document_paths_match_router() {
    let doc = openapi();
    let doc_paths = &doc.paths.paths;

    let mut expected: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for entry in ROUTE_REGISTRY {
        if let Some(p) = entry.openapi_path {
            expected.insert(p);
        }
    }

    for path in &expected {
        assert!(
            doc_paths.contains_key(*path),
            "route '{path}' is in ROUTE_REGISTRY but missing from the OpenAPI document; \
             add a #[utoipa::path] annotation and register it in ApiDoc paths()"
        );
    }

    assert_eq!(
        doc_paths.len(),
        expected.len(),
        "OpenAPI path count mismatch: registry declares {} unique paths, document has {}. \
         Update ROUTE_REGISTRY in src/routes/registry.rs when adding or removing routes.",
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

    let limit = document
        .pointer("/components/schemas/ServerMetaDto/properties/max_attachment_bytes")
        .expect("server metadata must advertise the optional attachment limit");

    assert_eq!(
        limit.get("type"),
        Some(&serde_json::json!(["integer", "null"]))
    );
    assert_eq!(limit.get("format"), Some(&Value::String("int64".into())));
    assert_eq!(limit.get("minimum"), Some(&serde_json::json!(0)));
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
