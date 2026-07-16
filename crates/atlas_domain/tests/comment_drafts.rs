#![allow(clippy::panic)]

use atlas_domain::{
    DomainError,
    entities::comments::{
        CommentDraftMetadata, comment_draft_create_digest_input,
        comment_draft_finalize_digest_input, comment_draft_upload_digest_input,
    },
};
use uuid::Uuid;

#[test]
fn metadata_normalization_trims_filename_and_canonicalizes_content_type() {
    let Ok(metadata) = CommentDraftMetadata::normalize(" report.pdf ", "Text/Plain") else {
        panic!("valid draft attachment metadata must normalize");
    };

    assert_eq!(metadata.file_name, "report.pdf");
    assert_eq!(metadata.content_type, "text/plain");
}

#[test]
fn metadata_normalization_rejects_unsafe_filename_and_content_type_parameters() {
    let unsafe_name = CommentDraftMetadata::normalize("nested/report.pdf", "text/plain");
    let parameterized_type =
        CommentDraftMetadata::normalize("report.pdf", "text/plain; charset=utf-8");

    assert!(matches!(unsafe_name, Err(DomainError::InvalidInput { .. })));
    assert!(matches!(
        parameterized_type,
        Err(DomainError::InvalidInput { .. })
    ));
}

#[test]
fn metadata_normalization_rejects_invalid_mime_token_grammar() {
    for content_type in [
        "text /plain",
        "text/ plain",
        "text/pla in",
        "text/pla\nin",
        "te(xt/plain",
        "text/pl@in",
    ] {
        let result = CommentDraftMetadata::normalize("report.pdf", content_type);

        assert!(
            matches!(result, Err(DomainError::InvalidInput { .. })),
            "{content_type:?} must be rejected as invalid MIME token grammar"
        );
    }
}

#[test]
fn create_digest_uses_raw_uuid_and_request_token_bytes() {
    let workspace_id = Uuid::from_u128(0x00112233445566778899aabbccddeeff);
    let draft_id = Uuid::from_u128(0xffeeddccbbaa99887766554433221100);

    let input = comment_draft_create_digest_input(workspace_id, draft_id, "create-token");

    assert_eq!(
        input,
        [
            b"comment-draft/v1".as_slice(),
            b"create".as_slice(),
            &16_u64.to_be_bytes(),
            workspace_id.as_bytes(),
            &16_u64.to_be_bytes(),
            draft_id.as_bytes(),
            &12_u64.to_be_bytes(),
            b"create-token".as_slice(),
        ]
        .concat()
    );
}

#[test]
fn upload_digest_uses_big_endian_size_and_raw_payload_digest() {
    let draft_id = Uuid::from_u128(0x00112233445566778899aabbccddeeff);
    let payload_digest = [0xab; 32];

    let input = comment_draft_upload_digest_input(
        draft_id,
        "upload-token",
        "report.pdf",
        "application/pdf",
        12,
        &payload_digest,
    );

    assert_eq!(
        input,
        [
            b"comment-draft/v1".as_slice(),
            b"upload".as_slice(),
            &16_u64.to_be_bytes(),
            draft_id.as_bytes(),
            &12_u64.to_be_bytes(),
            b"upload-token".as_slice(),
            &10_u64.to_be_bytes(),
            b"report.pdf".as_slice(),
            &15_u64.to_be_bytes(),
            b"application/pdf".as_slice(),
            &8_u64.to_be_bytes(),
            &12_u64.to_be_bytes(),
            &32_u64.to_be_bytes(),
            payload_digest.as_slice(),
        ]
        .concat()
    );
}

#[test]
fn finalize_digest_preserves_exact_utf8_body_bytes() {
    let draft_id = Uuid::from_u128(0x00112233445566778899aabbccddeeff);
    let body = "café\n";
    let request_digest = [0xcd; 32];

    let input = comment_draft_finalize_digest_input(draft_id, body, &request_digest);

    assert_eq!(
        input,
        [
            b"comment-draft/v1".as_slice(),
            b"finalize".as_slice(),
            &16_u64.to_be_bytes(),
            draft_id.as_bytes(),
            &body.len().to_be_bytes(),
            body.as_bytes(),
            &32_u64.to_be_bytes(),
            request_digest.as_slice(),
        ]
        .concat()
    );
}
