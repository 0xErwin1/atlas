use crate::ids::resource_ref::ResourceRef;

/// A document submitted to `SearchLexical` or `SearchSemantic` for indexing.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexedDocument {
    /// The resource this document represents.
    pub resource: ResourceRef,
    /// The text content to index.
    pub content: String,
}

/// A lexical (keyword) search query.
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalQuery {
    /// The raw query text.
    pub text: String,
    /// The maximum number of hits to return.
    pub limit: usize,
}

/// A semantic (embedding-based) search query.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticQuery {
    /// The raw query text to embed and search with.
    pub text: String,
    /// The maximum number of hits to return.
    pub limit: usize,
}

/// A single search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// The resource this hit refers to.
    pub resource: ResourceRef,
    /// The provider's relevance score for this hit.
    pub score: f32,
    /// An optional preview snippet of the matched content.
    pub snippet: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_hit_carries_no_product_coupling() {
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");
        let hit = SearchHit {
            resource: resource.clone(),
            score: 0.9,
            snippet: Some("...".to_string()),
        };

        assert_eq!(hit.resource, resource);
        assert_eq!(hit.score, 0.9);
        assert_eq!(hit.snippet.as_deref(), Some("..."));
    }
}
