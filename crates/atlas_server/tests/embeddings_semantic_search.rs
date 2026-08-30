use atlas_acta::ids::WorkspaceId;
use atlas_acta::semantic_search::EmbeddingInput;
use atlas_acta::semantic_search::EmbeddingProvider;
use atlas_acta::semantic_search::ResourceKind;
use atlas_acta::semantic_search::SemanticSearchSource;
use atlas_server::{
    config::{EmbeddingConfig, EmbeddingProviderKind, SCHEMA_EMBEDDING_DIMENSIONS},
    embeddings::{DeterministicEmbeddingProvider, OpenAiCompatibleEmbeddingProvider},
    semantic_indexer::{
        AttachmentText, ChecklistText, CommentText, DocumentIndexInput, SubtaskText,
        TaskIndexInput, aggregate_document_chunks, aggregate_task_chunks, chunk_semantic_text,
        document_content_hash, should_skip_embedding,
    },
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use std::{
    error::Error,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::net::TcpListener;
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

#[derive(Clone)]
struct EmbeddingsStub {
    batches: Arc<Mutex<Vec<usize>>>,
    attempts: Arc<AtomicUsize>,
    dimensions: usize,
    fail_first: usize,
}

async fn embeddings_stub_handler(
    State(stub): State<EmbeddingsStub>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    if stub.attempts.fetch_add(1, Ordering::SeqCst) < stub.fail_first {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({})));
    }

    let count = body
        .get("input")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    if let Ok(mut recorded) = stub.batches.lock() {
        recorded.push(count);
    }

    let data: Vec<serde_json::Value> = (0..count)
        .map(|_| serde_json::json!({ "embedding": vec![0.0_f32; stub.dimensions] }))
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "data": data })))
}

/// Serves the embeddings endpoint, recording each batch it is asked for and
/// failing the first `fail_first` requests with a retryable status.
async fn spawn_embeddings_stub(
    dimensions: usize,
    fail_first: usize,
) -> Result<(String, Arc<Mutex<Vec<usize>>>), Box<dyn Error>> {
    let batches: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let stub = EmbeddingsStub {
        batches: batches.clone(),
        attempts: Arc::new(AtomicUsize::new(0)),
        dimensions,
        fail_first,
    };

    let app = Router::new()
        .route("/embeddings", post(embeddings_stub_handler))
        .with_state(stub);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            eprintln!("embeddings stub stopped: {error}");
        }
    });

    Ok((base_url, batches))
}

fn stub_config(
    base_url: String,
    dimensions: usize,
    batch_size: usize,
    retries: u32,
) -> EmbeddingConfig {
    EmbeddingConfig {
        enabled: true,
        provider: EmbeddingProviderKind::OpenAiCompatible,
        model: "stub-embedding".to_owned(),
        dimensions,
        api_key: Some("stub-key".to_owned()),
        base_url,
        batch_size,
        timeout_ms: 5_000,
        retry_attempts: retries,
    }
}

fn inputs(count: usize) -> Vec<EmbeddingInput> {
    (0..count)
        .map(|idx| EmbeddingInput {
            text: format!("chunk {idx}"),
        })
        .collect()
}

#[tokio::test]
async fn openai_compatible_provider_splits_inputs_into_configured_batches()
-> Result<(), Box<dyn Error>> {
    let (base_url, batches) = spawn_embeddings_stub(SCHEMA_EMBEDDING_DIMENSIONS, 0).await?;
    let provider = OpenAiCompatibleEmbeddingProvider::new(stub_config(
        base_url,
        SCHEMA_EMBEDDING_DIMENSIONS,
        2,
        0,
    ))?;

    let vectors = provider.embed(&inputs(5)).await?;

    assert_eq!(vectors.len(), 5);
    let recorded = batches
        .lock()
        .map_err(|_| io::Error::other("stub batch log poisoned"))?
        .clone();
    assert_eq!(
        recorded,
        vec![2, 2, 1],
        "ATLAS_EMBEDDINGS_BATCH_SIZE must bound each request"
    );
    Ok(())
}

#[tokio::test]
async fn openai_compatible_provider_retries_a_transient_provider_failure()
-> Result<(), Box<dyn Error>> {
    let (base_url, batches) = spawn_embeddings_stub(SCHEMA_EMBEDDING_DIMENSIONS, 2).await?;
    let provider = OpenAiCompatibleEmbeddingProvider::new(stub_config(
        base_url,
        SCHEMA_EMBEDDING_DIMENSIONS,
        64,
        2,
    ))?;

    let vectors = provider.embed(&inputs(1)).await?;

    assert_eq!(vectors.len(), 1);
    assert_eq!(
        batches
            .lock()
            .map_err(|_| io::Error::other("stub batch log poisoned"))?
            .len(),
        1,
        "only the attempt that succeeded reaches the handler body"
    );
    Ok(())
}

#[tokio::test]
async fn openai_compatible_provider_gives_up_after_the_configured_retries()
-> Result<(), Box<dyn Error>> {
    let (base_url, _) = spawn_embeddings_stub(SCHEMA_EMBEDDING_DIMENSIONS, 10).await?;
    let provider = OpenAiCompatibleEmbeddingProvider::new(stub_config(
        base_url,
        SCHEMA_EMBEDDING_DIMENSIONS,
        64,
        1,
    ))?;

    assert!(provider.embed(&inputs(1)).await.is_err());
    Ok(())
}

#[test]
fn semantic_search_embeddings_config_requires_an_explicit_provider_when_enabled() {
    let inherited = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        _ => None,
    });

    let error = inherited.err().unwrap_or_default();
    assert!(
        error.contains("ATLAS_EMBEDDINGS_PROVIDER"),
        "enabling embeddings without naming a provider must fail loudly: {error}"
    );

    let asked_for_deterministic = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        "ATLAS_EMBEDDINGS_PROVIDER" => Some("deterministic".to_owned()),
        _ => None,
    });
    assert!(
        asked_for_deterministic
            .is_ok_and(|cfg| cfg.provider == EmbeddingProviderKind::Deterministic),
        "the deterministic provider stays available when it is asked for by name"
    );

    let disabled = EmbeddingConfig::from_env_vars(|_| None);
    assert!(
        disabled.is_ok(),
        "a deployment with embeddings off needs no provider"
    );
}

#[test]
fn semantic_search_embeddings_config_rejects_dimensions_the_schema_cannot_store() {
    let mismatched = EmbeddingConfig::from_env_vars(|name| match name {
        "ATLAS_EMBEDDINGS_ENABLED" => Some("true".to_owned()),
        "ATLAS_EMBEDDINGS_PROVIDER" => Some("openai_compatible".to_owned()),
        "ATLAS_EMBEDDINGS_MODEL" => Some("text-embedding-3-small".to_owned()),
        "ATLAS_EMBEDDINGS_DIMENSIONS" => Some("768".to_owned()),
        "ATLAS_EMBEDDINGS_API_KEY" => Some("secret".to_owned()),
        _ => None,
    });

    let error = mismatched.err().unwrap_or_default();

    assert!(
        error.contains("768") && error.contains("search_embeddings"),
        "startup error must name the configured width and the column it cannot fit: {error}"
    );
    assert_eq!(SCHEMA_EMBEDDING_DIMENSIONS, 1536);
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
