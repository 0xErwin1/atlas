use atlas_domain::{
    ids::{DocumentId, WorkspaceId},
    permissions::Principal,
    semantic_search::{
        EmbeddingInput, EmbeddingProvider, ResourceKind, SemanticSearchAfter, SemanticSearchHit,
        SemanticSearchQuery, SemanticSearchSource, SemanticSearchTypeFilter,
    },
};
use std::error::Error;
use uuid::Uuid;

#[test]
fn semantic_search_cursor_order_is_deterministic_for_equal_similarity() {
    let low = Uuid::from_u128(1);
    let high = Uuid::from_u128(2);

    let first = SemanticSearchAfter::new(0.8125, ResourceKind::Document, high);
    let second = SemanticSearchAfter::new(0.8125, ResourceKind::Document, low);

    assert!(first.sort_tuple() > second.sort_tuple());
    assert_eq!(first.resource_id, high);
}

#[test]
fn semantic_search_query_carries_permission_safe_lookup_inputs() {
    let workspace_id = WorkspaceId(Uuid::from_u128(10));
    let principal = Principal::User(atlas_domain::ids::UserId(Uuid::from_u128(20)));
    let query = SemanticSearchQuery::new(
        workspace_id,
        principal.clone(),
        "retention policy".to_owned(),
        SemanticSearchTypeFilter::documents(),
        25,
        None,
        false,
        true,
        false,
    );

    assert_eq!(query.workspace_id, workspace_id);
    assert_eq!(query.principal, principal);
    assert!(query.type_filter.documents);
    assert!(!query.type_filter.tasks);
    assert!(query.may_read_documents);
    assert!(!query.may_read_tasks);
}

#[test]
fn semantic_search_hit_is_compact_and_hydration_friendly() {
    let id = DocumentId(Uuid::from_u128(30));
    let hit = SemanticSearchHit {
        kind: ResourceKind::Document,
        id: id.0,
        readable_id: None,
        title: "Policy".to_owned(),
        project_slug: Some("ops".to_owned()),
        column_name: None,
        similarity: 0.91,
        source: SemanticSearchSource::Content,
        excerpt: "retention requirements".to_owned(),
    };

    assert_eq!(hit.id, id.0);
    assert_eq!(hit.kind, ResourceKind::Document);
    assert!(hit.excerpt.len() < 240);
}

#[tokio::test]
async fn semantic_search_embedding_provider_contract_returns_model_and_dimensions()
-> Result<(), Box<dyn Error>> {
    struct StaticProvider;

    #[async_trait::async_trait]
    impl EmbeddingProvider for StaticProvider {
        async fn embed(
            &self,
            inputs: &[EmbeddingInput],
        ) -> Result<Vec<Vec<f32>>, atlas_domain::DomainError> {
            let first = inputs
                .first()
                .ok_or_else(|| atlas_domain::DomainError::InvalidInput {
                    message: "missing embedding input".to_owned(),
                })?;
            assert_eq!(first.text, "hello");
            Ok(vec![vec![0.1, 0.2, 0.3]])
        }

        fn model(&self) -> &str {
            "test-model"
        }

        fn dimensions(&self) -> usize {
            3
        }
    }

    let provider = StaticProvider;
    let embeddings = provider
        .embed(&[EmbeddingInput {
            text: "hello".to_owned(),
        }])
        .await?;

    assert_eq!(provider.model(), "test-model");
    assert_eq!(provider.dimensions(), 3);
    assert_eq!(embeddings, vec![vec![0.1, 0.2, 0.3]]);
    Ok(())
}
