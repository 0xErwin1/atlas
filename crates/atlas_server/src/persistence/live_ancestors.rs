use sea_orm::sea_query::{Expr, SimpleExpr};

pub(crate) fn live_project(project_id: &str) -> SimpleExpr {
    Expr::cust(project_is_live_sql(project_id))
}

pub(crate) fn live_folder_chain(folder_id: &str) -> SimpleExpr {
    Expr::cust(folder_chain_is_live_sql(folder_id))
}

pub(crate) fn live_board_chain(board_id: &str) -> SimpleExpr {
    Expr::cust(board_chain_is_live_sql(board_id))
}

pub(crate) fn live_task_chain(task_id: &str) -> SimpleExpr {
    Expr::cust(task_chain_is_live_sql(task_id))
}

pub(crate) fn live_document_chain(document_id: &str) -> SimpleExpr {
    Expr::cust(document_chain_is_live_sql(document_id))
}

pub(crate) fn live_comment_chain(comment_id: &str) -> SimpleExpr {
    Expr::cust(comment_chain_is_live_sql(comment_id))
}

pub(crate) fn board_chain_is_live_sql(board_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            SELECT 1 FROM boards live_board \
            WHERE live_board.id = {board_id} \
              AND (\
                    live_board.deleted_at IS NOT NULL \
                    OR NOT ({project_live}) \
                    OR NOT ({folder_live})\
              )\
        )",
        project_live = project_is_live_sql("live_board.project_id"),
        folder_live = folder_chain_is_live_sql("live_board.folder_id"),
    )
}

pub(crate) fn document_chain_is_live_sql(document_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            SELECT 1 FROM documents live_document \
            WHERE live_document.id = {document_id} \
              AND (\
                    live_document.deleted_at IS NOT NULL \
                    OR NOT ({project_live}) \
                    OR NOT ({folder_live})\
              )\
        )",
        project_live = project_is_live_sql("live_document.project_id"),
        folder_live = folder_chain_is_live_sql("live_document.folder_id"),
    )
}

pub(crate) fn task_chain_is_live_sql(task_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            SELECT 1 FROM tasks live_task \
            WHERE live_task.id = {task_id} \
              AND (\
                    live_task.deleted_at IS NOT NULL \
                    OR NOT ({board_live})\
              )\
        )",
        board_live = board_chain_is_live_sql("live_task.board_id"),
    )
}

pub(crate) fn comment_chain_is_live_sql(comment_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            SELECT 1 FROM comments live_comment \
            WHERE live_comment.id = {comment_id} \
              AND (\
                    live_comment.deleted_at IS NOT NULL \
                    OR (live_comment.task_id IS NOT NULL \
                        AND NOT ({task_live})) \
                    OR (live_comment.document_id IS NOT NULL \
                        AND NOT ({document_live}))\
              )\
        )",
        task_live = task_chain_is_live_sql("live_comment.task_id"),
        document_live = document_chain_is_live_sql("live_comment.document_id"),
    )
}

pub(crate) fn project_is_live_sql(project_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            SELECT 1 FROM projects live_project \
            WHERE live_project.id = {project_id} \
              AND live_project.deleted_at IS NOT NULL\
        )"
    )
}

pub(crate) fn folder_chain_is_live_sql(folder_id: &str) -> String {
    format!(
        "NOT EXISTS (\
            WITH RECURSIVE folder_ancestors AS (\
                SELECT id, parent_folder_id, deleted_at, ARRAY[id] AS path \
                FROM folders \
                WHERE id = {folder_id} \
                UNION ALL \
                SELECT parent.id, parent.parent_folder_id, parent.deleted_at, \
                       ancestors.path || parent.id \
                FROM folders parent \
                JOIN folder_ancestors ancestors \
                  ON parent.id = ancestors.parent_folder_id \
                WHERE NOT parent.id = ANY(ancestors.path)\
            ) \
            SELECT 1 FROM folder_ancestors \
            WHERE deleted_at IS NOT NULL\
        )"
    )
}
