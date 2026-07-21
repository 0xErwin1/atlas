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
