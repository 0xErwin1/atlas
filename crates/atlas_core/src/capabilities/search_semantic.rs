use async_trait::async_trait;

use crate::ids::resource_ref::ResourceRef;

use super::error::CapabilityError;
use super::search::{IndexedDocument, SearchHit, SemanticQuery};

/// Embedding-based search over data the caller has already authorized.
///
/// Method signatures carry no principal, role, or authorization flag
/// (SHELL-CAP-3): authorization happens before a call reaches this trait.
#[async_trait]
pub trait SearchSemantic: Send + Sync {
    /// Indexes or re-indexes `document`, replacing any prior embedding.
    async fn upsert(&self, document: IndexedDocument) -> Result<(), CapabilityError>;

    /// Removes `resource` from the index, if present.
    async fn remove(&self, resource: &ResourceRef) -> Result<(), CapabilityError>;

    /// Runs `query` against the index.
    async fn search(&self, query: &SemanticQuery) -> Result<Vec<SearchHit>, CapabilityError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::test_support::block_on;
    use std::sync::Mutex;

    struct StubIndex {
        documents: Mutex<Vec<IndexedDocument>>,
    }

    impl StubIndex {
        fn new() -> Self {
            Self {
                documents: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SearchSemantic for StubIndex {
        async fn upsert(&self, document: IndexedDocument) -> Result<(), CapabilityError> {
            self.documents.lock().expect("lock").push(document);
            Ok(())
        }

        async fn remove(&self, resource: &ResourceRef) -> Result<(), CapabilityError> {
            self.documents
                .lock()
                .expect("lock")
                .retain(|doc| &doc.resource != resource);
            Ok(())
        }

        async fn search(&self, query: &SemanticQuery) -> Result<Vec<SearchHit>, CapabilityError> {
            let hits = self
                .documents
                .lock()
                .expect("lock")
                .iter()
                .filter(|doc| doc.content.contains(&query.text))
                .map(|doc| SearchHit {
                    resource: doc.resource.clone(),
                    score: 1.0,
                    snippet: None,
                })
                .collect();

            Ok(hits)
        }
    }

    #[test]
    fn search_semantic_is_object_safe() {
        let _: Option<Box<dyn SearchSemantic>> = None;
    }

    #[test]
    fn upsert_then_search_finds_the_document() {
        let index: Box<dyn SearchSemantic> = Box::new(StubIndex::new());
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");

        block_on(index.upsert(IndexedDocument {
            resource: resource.clone(),
            content: "hello world".to_string(),
        }))
        .expect("upsert succeeds");

        let hits = block_on(index.search(&SemanticQuery {
            text: "hello".to_string(),
            limit: 10,
        }))
        .expect("search succeeds");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].resource, resource);
    }

    #[test]
    fn remove_excludes_future_results() {
        let index: Box<dyn SearchSemantic> = Box::new(StubIndex::new());
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");

        block_on(index.upsert(IndexedDocument {
            resource: resource.clone(),
            content: "hello world".to_string(),
        }))
        .expect("upsert succeeds");
        block_on(index.remove(&resource)).expect("remove succeeds");

        let hits = block_on(index.search(&SemanticQuery {
            text: "hello".to_string(),
            limit: 10,
        }))
        .expect("search succeeds");

        assert!(hits.is_empty());
    }
}
