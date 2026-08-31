// R1 scaffolding: these raw-SQL "is this ancestor chain still live" helpers
// now live in `atlas_acta_postgres::live_ancestors` (S4 PR7), moved alongside
// the documents/boards_tasks/comments repos that call them. Re-exporting them
// here keeps every existing `crate::persistence::live_ancestors::*` call site
// (`authz/authorized.rs`, and the not-yet-moved `workspace_attachments.rs`,
// `search.rs`, `semantic_search.rs`, `workspace_core.rs`) unaffected.
pub(crate) use atlas_acta_postgres::live_ancestors::*;
