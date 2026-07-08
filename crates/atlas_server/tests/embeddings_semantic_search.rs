use atlas_domain::{
    ids::WorkspaceId,
    semantic_search::{EmbeddingInput, EmbeddingProvider, ResourceKind, SemanticSearchSource},
};
use atlas_server::{
    config::{EmbeddingConfig, EmbeddingProviderKind},
    embeddings::DeterministicEmbeddingProvider,
    semantic_indexer::{
        AttachmentText, ChecklistText, CommentText, DocumentIndexInput, SubtaskText,
        TaskIndexInput, aggregate_document_chunks, aggregate_task_chunks, chunk_semantic_text,
        document_content_hash, should_skip_embedding,
    },
};
use std::{error::Error, io};
use uuid::Uuid;

#[tokio::test]
async fn semantic_search_embeddings_deterministic_provider_guards_dimensions()
-> Result<(), Box<dyn Error>> {
    let provider = DeterministicEmbeddingProvider::new("test-embedding", 4)?;

    let vectors = provider
        .embed(&[
            EmbeddingInput {
                text: "retention policy".to_owned(),
            },
            EmbeddingInput {
                text: "incident review".to_owned(),
            },
        ])
        .await?;

    let first = vectors
        .first()
        .ok_or_else(|| io::Error::other("missing first embedding"))?;
    let second = vectors
        .get(1)
        .ok_or_else(|| io::Error::other("missing second embedding"))?;
    let repeated = provider
        .embed(&[EmbeddingInput {
            text: "retention policy".to_owned(),
        }])
        .await?;
    let repeated_first = repeated
        .first()
        .ok_or_else(|| io::Error::other("missing repeated embedding"))?;

    assert_eq!(provider.model(), "test-embedding");
    assert_eq!(provider.dimensions(), 4);
    assert_eq!(vectors.len(), 2);
    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4);
    assert_eq!(first, repeated_first);
    assert_ne!(first, second);
    assert!(DeterministicEmbeddingProvider::new("bad", 0).is_err());
    Ok(())
}

#[test]
fn semantic_search_embeddings_openai_compatible_config_requires_key_and_dimensions()
-> Result<(), Box<dyn Error>> {
    let cfg = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        "ATLAS_EMBEDDINGS_PROVIDER" => Some("openai_compatible".to_owned()),
        "ATLAS_EMBEDDINGS_MODEL" => Some("text-embedding-3-small".to_owned()),
        "ATLAS_EMBEDDINGS_DIMENSIONS" => Some("1536".to_owned()),
        "ATLAS_EMBEDDINGS_API_KEY" => Some("secret".to_owned()),
        _ => None,
    })
    .map_err(io::Error::other)?;

    assert!(cfg.enabled);
    assert_eq!(cfg.provider, EmbeddingProviderKind::OpenAiCompatible);
    assert_eq!(cfg.model, "text-embedding-3-small");
    assert_eq!(cfg.dimensions, 1536);

    let missing_key = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        "ATLAS_EMBEDDINGS_PROVIDER" => Some("openai_compatible".to_owned()),
        "ATLAS_EMBEDDINGS_MODEL" => Some("text-embedding-3-small".to_owned()),
        "ATLAS_EMBEDDINGS_DIMENSIONS" => Some("1536".to_owned()),
        _ => None,
    });
    assert!(missing_key.is_err());

    let bad_dimensions = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        "ATLAS_EMBEDDINGS_PROVIDER" => Some("openai_compatible".to_owned()),
        "ATLAS_EMBEDDINGS_MODEL" => Some("text-embedding-3-small".to_owned()),
        "ATLAS_EMBEDDINGS_DIMENSIONS" => Some("0".to_owned()),
        "ATLAS_EMBEDDINGS_API_KEY" => Some("secret".to_owned()),
        _ => None,
    });
    assert!(bad_dimensions.is_err());
    Ok(())
}

#[test]
fn semantic_search_aggregation_includes_task_inherited_visible_text() -> Result<(), Box<dyn Error>>
{
    let workspace_id = WorkspaceId(Uuid::from_u128(1));
    let task_id = Uuid::from_u128(2);
    let chunks = aggregate_task_chunks(TaskIndexInput {
        workspace_id,
        task_id,
        readable_id: "ATL-42".to_owned(),
        title: "Quarterly planning".to_owned(),
        description: "Prepare roadmap".to_owned(),
        labels: vec!["strategy".to_owned(), "retention".to_owned()],
        comments: vec![CommentText {
            body: "Customer asks about long-term memory".to_owned(),
        }],
        attachments: vec![AttachmentText {
            file_name: "policy-retention.pdf".to_owned(),
        }],
        checklist_items: vec![ChecklistText {
            title: "Confirm audit logging".to_owned(),
        }],
        subtasks: vec![SubtaskText {
            readable_id: "ATL-43".to_owned(),
            title: "Draft incident review".to_owned(),
            description: "Summarize semantic retrieval".to_owned(),
            checklist_items: vec![ChecklistText {
                title: "Notify support".to_owned(),
            }],
        }],
        max_chunk_chars: 1000,
    });

    assert_eq!(chunks.len(), 1);
    let chunk = chunks
        .first()
        .ok_or_else(|| io::Error::other("missing task aggregate chunk"))?;
    assert_eq!(chunk.workspace_id, workspace_id);
    assert_eq!(chunk.kind, ResourceKind::Task);
    assert_eq!(chunk.resource_id, task_id);
    assert_eq!(chunk.source, SemanticSearchSource::Aggregate);
    assert!(chunk.text.contains("ATL-42"));
    assert!(chunk.text.contains("policy-retention.pdf"));
    assert!(chunk.text.contains("Confirm audit logging"));
    assert!(chunk.text.contains("ATL-43"));
    assert!(chunk.content_hash.len() >= 32);
    Ok(())
}

#[test]
fn semantic_search_chunking_hashes_and_skips_unchanged_content() -> Result<(), Box<dyn Error>> {
    let workspace_id = WorkspaceId(Uuid::from_u128(10));
    let document_id = Uuid::from_u128(11);
    let chunks = aggregate_document_chunks(DocumentIndexInput {
        workspace_id,
        document_id,
        title: "Runbook".to_owned(),
        content: "alpha beta gamma delta epsilon zeta eta theta".to_owned(),
        comments: vec![CommentText {
            body: "commentary for recovery".to_owned(),
        }],
        attachments: vec![AttachmentText {
            file_name: "restore-plan.md".to_owned(),
        }],
        max_chunk_chars: 24,
    });

    assert!(chunks.len() > 1);
    assert!(
        chunks
            .iter()
            .any(|chunk| chunk.text.contains("restore-plan.md"))
    );
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.kind == ResourceKind::Document)
    );
    let first = chunks
        .first()
        .ok_or_else(|| io::Error::other("missing first document chunk"))?;
    let second = chunks
        .get(1)
        .ok_or_else(|| io::Error::other("missing second document chunk"))?;
    assert_eq!(first.chunk_ordinal, 0);
    assert_eq!(second.chunk_ordinal, 1);

    let hash = document_content_hash("Runbook", "alpha beta");
    assert!(should_skip_embedding(&hash, Some(hash.as_str())));
    assert!(!should_skip_embedding(&hash, Some("different")));
    assert!(!should_skip_embedding(&hash, None));

    let direct_chunks = chunk_semantic_text(
        workspace_id,
        ResourceKind::Document,
        document_id,
        SemanticSearchSource::Content,
        "one two three four five six",
        13,
    );
    assert_eq!(direct_chunks.len(), 2);
    assert!(direct_chunks.iter().all(|chunk| chunk.text.len() <= 13));
    Ok(())
}
