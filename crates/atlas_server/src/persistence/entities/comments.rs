// R1 scaffolding: `comment`, `comment_link`, `comment_link_event` (and their
// `_from` conversions) now live in `atlas_acta_postgres::entities::comments`
// (S4 PR4). Re-exporting them here keeps every existing
// `crate::persistence::entities::comments::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::comments::*;

// `comment_attachment_draft`/`comment_attachment_draft_upload` (and their
// `_from` conversions) moved into `atlas_acta_postgres::entities::documents`
// in S4 PR2/PR3, not `entities::comments` — the table lives in the
// documents-family module, not the comments-family one. Re-exported here too
// so `crate::persistence::entities::comments::comment_attachment_draft*` call
// sites keep resolving.
pub use atlas_acta_postgres::entities::documents::{
    comment_attachment_draft, comment_attachment_draft_from, comment_attachment_draft_upload,
    comment_attachment_draft_upload_from,
};
