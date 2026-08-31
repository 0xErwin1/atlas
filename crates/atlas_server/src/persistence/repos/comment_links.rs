//! Facade: `PgCommentLinkRepo` moved to
//! `atlas_acta_postgres::repos::comment_links` (S4 PR8). Re-exported here so
//! existing `crate::persistence::repos::*` call sites keep resolving
//! unchanged. `CommentMutationFault` moved alongside it (it is threaded into
//! `PgCommentLinkRepo::replace_for_comment_with_fault_in` as a parameter, so
//! it must live in the same crate as that method) — `services::comment_service`
//! now imports it from here instead of defining it, keeping
//! `services::CommentMutationFault`'s outward path unchanged.

pub use atlas_acta_postgres::repos::comment_links::{CommentMutationFault, PgCommentLinkRepo};
