// R1 scaffolding: the board/task/task_reference/task_assignee/
// task_checklist_item/task_activity repos (`PgBoardRepo`, `PgTaskRepo`,
// `PgTaskReferenceRepo`, `PgTaskAssigneeRepo`, `PgTaskChecklistRepo`,
// `PgTaskActivityRepo`, their trait impls, and `resequence_column`) now live
// in `atlas_acta_postgres::repos::boards_tasks` (S4 PR7). Re-exporting them
// here keeps every existing `crate::persistence::repos::*` call site
// unaffected.
pub use atlas_acta_postgres::repos::boards_tasks::{
    BoardRepo, PgBoardRepo, PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo,
    PgTaskReferenceRepo, PgTaskRepo, TaskActivityRepo, TaskAssigneeRepo, TaskChecklistRepo,
    TaskReferenceRepo, TaskRepo, resequence_column,
};
