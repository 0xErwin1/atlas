use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260624_000024_groups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE groups (
                id           UUID        PRIMARY KEY,
                workspace_id UUID        NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                name         TEXT        NOT NULL,
                created_by   UUID        NOT NULL REFERENCES users(id),
                created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at   TIMESTAMPTZ NULL
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX groups_workspace_name_uidx
            ON groups (workspace_id, lower(name))
            WHERE deleted_at IS NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE group_members (
                group_id   UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (group_id, user_id)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD COLUMN group_id UUID REFERENCES groups(id) ON DELETE CASCADE",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_principal_xor",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_principal_xor \
             CHECK (num_nonnulls(user_id, api_key_id, group_id) = 1)",
        )
        .await?;

        conn.execute_unprepared("DROP INDEX permission_grants_uq")
            .await?;

        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX permission_grants_uq
            ON permission_grants (workspace_id, user_id, api_key_id, group_id, project_id, folder_id, document_id, board_id)
            NULLS NOT DISTINCT
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_group_ws_idx
            ON permission_grants (workspace_id, group_id)
            WHERE group_id IS NOT NULL
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP INDEX IF EXISTS permission_grants_group_ws_idx")
            .await?;

        conn.execute_unprepared("DROP INDEX IF EXISTS permission_grants_uq")
            .await?;

        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX permission_grants_uq
            ON permission_grants (workspace_id, user_id, api_key_id, project_id, folder_id, document_id, board_id)
            NULLS NOT DISTINCT
            "#,
        )
        .await?;

        conn.execute_unprepared("ALTER TABLE permission_grants DROP CONSTRAINT IF EXISTS permission_grants_principal_xor")
            .await?;

        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_principal_xor \
             CHECK (num_nonnulls(user_id, api_key_id) = 1)",
        )
        .await?;

        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN IF EXISTS group_id")
            .await?;

        conn.execute_unprepared("DROP TABLE IF EXISTS group_members")
            .await?;

        conn.execute_unprepared("DROP TABLE IF EXISTS groups CASCADE")
            .await?;

        Ok(())
    }
}
