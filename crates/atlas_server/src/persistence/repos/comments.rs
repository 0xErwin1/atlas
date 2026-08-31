// R1 scaffolding: the comment repo (`PgCommentRepo`, its `CommentRepo` trait
// impl, and its `*_in` transaction-scoped helpers) now lives in
// `atlas_acta_postgres::repos::comments` (S4 PR7). Re-exporting it here keeps
// every existing `crate::persistence::repos::*` call site unaffected.
pub use atlas_acta_postgres::repos::comments::{CommentRepo, PgCommentRepo};
