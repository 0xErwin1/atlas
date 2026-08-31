//! Facade: `PgSemanticIndexWriter`/`PgSemanticSearchRepo` moved to
//! `atlas_acta_postgres::repos::semantic_search` (S4 PR8). Re-exported here
//! so existing `crate::persistence::repos::*` call sites (including
//! `semantic_indexer.rs`, which stays server-side) keep resolving unchanged.

pub use atlas_acta_postgres::repos::semantic_search::{
    PgSemanticIndexWriter, PgSemanticSearchRepo, semantic_search_sql,
};
