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
    "TrashKindDto",
    "TrashItemDto",
    "RestoreTrashItemRequest",
    "Page_TrashItemDto",
    "PurgeStatusDto",
    "PurgeTrashItemRequest",
    "PurgeStatusDtoResponse",
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
