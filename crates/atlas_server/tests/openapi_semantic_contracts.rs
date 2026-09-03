#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Per-operation semantic OpenAPI assertions preserved from the retired
//! `openapi_drift.rs` (D5): `EXPECTED_SCHEMAS`/`EXPECTED_OPENAPI_PATHS` and
//! their two set-membership tests were replaced by `openapi_zero_drift.rs`'s
//! registry-derived, `HashSet`-based comparison (never a hand-maintained
//! literal list). The targeted, per-operation assertions below have no
//! registry-derivable equivalent — they pin specific documented shapes
//! (status codes, schema `$ref`s, header requirements) that only the
//! document itself can state — so they are preserved verbatim, unchanged in
//! substance, just relocated out of the file that also carried the retired
//! literals.
//!
//! Path-key update, `v2-e3-s6` PR1 (D1.3): every document-key literal below
//! is built through `support::path::document_path("acta", "<rel>")`, never a
//! hand-typed `/api/...` string — all 24 relative paths this file names are
//! `acta`-owned, unchanged from before this slice; only how each key is
//! built changed, to track `document()`'s V2 re-key.

mod support;

use atlas_server::routes::openapi::openapi;
use serde_json::Value;
use support::path::document_path;

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

    let trash_path = document_path("acta", "/admin/trash");
    assert_operation_statuses(&document, &trash_path, "get", &[200, 400, 401, 403]);
    let list = operation(&document, &trash_path, "get");
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
        &document_path("acta", "/admin/trash/restore"),
        "post",
        &[204, 401, 403, 404, 409],
    );
    assert_operation_statuses(
        &document,
        &document_path("acta", "/admin/trash/purge"),
        "post",
        &[202, 204, 400, 401, 403, 404],
    );
    assert_operation_statuses(
        &document,
        &document_path("acta", "/admin/trash/purges/{operation_id}"),
        "get",
        &[200, 401, 403, 404],
    );
}

#[test]
fn task_attachment_rename_operation_documents_typed_contract() {
    let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
    let path = document_path(
        "acta",
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
    );

    assert_operation_statuses(&document, &path, "patch", &[200, 401, 403, 404, 422]);
    let patch = operation(&document, &path, "patch");
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
        &document_path("acta", "/workspaces/{ws}/documents/{slug}/compact"),
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
    let path = document_path(
        "acta",
        "/workspaces/{ws}/tasks/{readable_id}/references/batch",
    );
    let batch = operation(&document, &path, "post");

    assert_operation_statuses(&document, &path, "post", &[200, 413, 422]);
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
    let path = document_path("acta", "/workspaces/{ws}/documents/moves/batch");
    let batch = operation(&document, &path, "post");

    assert_operation_statuses(&document, &path, "post", &[200, 413, 422]);
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

    for (parent_relative, attachment_relative) in [
        (
            "/workspaces/{ws}/tasks/{readable_id}",
            "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
        ),
        (
            "/workspaces/{ws}/documents/{slug}",
            "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        ),
    ] {
        let draft_relative = format!("{parent_relative}/comment-drafts");
        let upload_relative = format!("{draft_relative}/{{draft_id}}/attachments");
        let cancel_relative = format!("{draft_relative}/{{draft_id}}");
        let comments_relative = format!("{parent_relative}/comments");
        let comment_attachments_relative =
            format!("{parent_relative}/comments/{{comment_id}}/attachments");
        let comment_attachment_item_relative =
            format!("{parent_relative}/comments/{{comment_id}}/attachments/{{attachment_id}}");

        let draft_path = document_path("acta", &draft_relative);
        let upload_path = document_path("acta", &upload_relative);
        let cancel_path = document_path("acta", &cancel_relative);
        let comments_path = document_path("acta", &comments_relative);
        let comment_attachments_path = document_path("acta", &comment_attachments_relative);
        let comment_attachment_item_path = document_path("acta", &comment_attachment_item_relative);
        let attachment_path = document_path("acta", attachment_relative);

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
            &comments_path,
            "post",
            &[200, 201, 404, 409, 410, 422],
        );
        assert_operation_statuses(
            &document,
            &comment_attachments_path,
            "get",
            &[200, 404, 410],
        );
        assert_operation_statuses(
            &document,
            &comment_attachment_item_path,
            "delete",
            &[204, 404, 410],
        );
        assert_operation_statuses(&document, &attachment_path, "get", &[200, 404, 410]);

        assert_header_parameter(operation(&document, &draft_path, "post"), "x-create-token");
        assert_header_parameter(operation(&document, &upload_path, "post"), "x-upload-token");

        let create = operation(&document, &comments_path, "post");
        assert!(
            create
                .pointer("/requestBody/content/application~1json/schema/$ref")
                .is_some_and(|schema| schema == "#/components/schemas/CreateCommentRequest"),
            "{parent_relative} comment creation must use the shared CreateCommentRequest schema"
        );

        let get = operation(&document, &attachment_path, "get");
        for header in [
            "Content-Type",
            "Content-Disposition",
            "X-Content-Type-Options",
        ] {
            assert!(
                get.pointer(&format!("/responses/200/headers/{header}"))
                    .is_some(),
                "{attachment_relative} must document its {header} response header"
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
        document_path("acta", "/workspaces/{ws}/tasks/{readable_id}/comments"),
        document_path("acta", "/workspaces/{ws}/documents/{slug}/comments"),
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
        document_path("acta", "/workspaces/{ws}/tasks/{readable_id}/comments"),
        document_path("acta", "/workspaces/{ws}/documents/{slug}/comments"),
    ] {
        let get = operation(&document, &path, "get");

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
        &document_path(
            "acta",
            "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        ),
        &document_path(
            "acta",
            "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
        ),
        &document_path(
            "acta",
            "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
        ),
    );
    assert_attachment_lifecycle(
        &document,
        &document_path(
            "acta",
            "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        ),
        &document_path(
            "acta",
            "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        ),
        &document_path(
            "acta",
            "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        ),
    );

    let document_upload = operation(
        &document,
        &document_path(
            "acta",
            "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        ),
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
