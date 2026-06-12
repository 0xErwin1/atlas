use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260612_000005_permissions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Add visibility columns to shareable resource tables.
        for table in &["projects", "folders", "documents", "boards"] {
            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} ADD COLUMN visibility TEXT NOT NULL DEFAULT 'workspace' \
                 CHECK (visibility IN ('private','workspace','public'))"
            ))
            .await?;

            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} ADD COLUMN visibility_role TEXT DEFAULT 'editor' \
                 CHECK (visibility_role IN ('viewer','editor'))"
            ))
            .await?;
        }

        // Add disabled_at to users.
        conn.execute_unprepared("ALTER TABLE users ADD COLUMN disabled_at TIMESTAMPTZ NULL")
            .await?;

        // Drop api_keys.role (grants replace it).
        conn.execute_unprepared("ALTER TABLE api_keys DROP COLUMN role")
            .await?;

        // Create permission_grants table.
        conn.execute_unprepared(
            r#"
            CREATE TABLE permission_grants (
                id                   UUID        PRIMARY KEY,
                workspace_id         UUID        NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                user_id              UUID        REFERENCES users(id) ON DELETE CASCADE,
                api_key_id           UUID        REFERENCES api_keys(id) ON DELETE CASCADE,
                project_id           UUID        REFERENCES projects(id) ON DELETE CASCADE,
                folder_id            UUID        REFERENCES folders(id) ON DELETE CASCADE,
                document_id          UUID        REFERENCES documents(id) ON DELETE CASCADE,
                board_id             UUID        REFERENCES boards(id) ON DELETE CASCADE,
                role                 TEXT        NOT NULL CHECK (role IN ('viewer','editor','admin')),
                created_by_user_id   UUID        REFERENCES users(id) ON DELETE SET NULL,
                created_by_api_key_id UUID       REFERENCES api_keys(id) ON DELETE SET NULL,
                created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT permission_grants_principal_xor
                    CHECK (num_nonnulls(user_id, api_key_id) = 1),
                CONSTRAINT permission_grants_target_at_most_one
                    CHECK (num_nonnulls(project_id, folder_id, document_id, board_id) <= 1)
            )
            "#,
        )
        .await?;

        // Unique index (one grant per principal-resource pair); NULLS NOT DISTINCT treats NULLs
        // as equal so that (ws_id, user_id=X, all_targets=NULL) is a unique workspace-scope grant.
        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX permission_grants_uq
            ON permission_grants (workspace_id, user_id, api_key_id, project_id, folder_id, document_id, board_id)
            NULLS NOT DISTINCT
            "#,
        )
        .await?;

        // Hot-path lookup indexes: one per principal type.
        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_user_ws_idx
            ON permission_grants (workspace_id, user_id)
            WHERE user_id IS NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_api_key_ws_idx
            ON permission_grants (workspace_id, api_key_id)
            WHERE api_key_id IS NOT NULL
            "#,
        )
        .await?;

        // Reverse lookup indexes per target column (for list/unshare operations).
        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_project_idx
            ON permission_grants (project_id)
            WHERE project_id IS NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_folder_idx
            ON permission_grants (folder_id)
            WHERE folder_id IS NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_document_idx
            ON permission_grants (document_id)
            WHERE document_id IS NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE INDEX permission_grants_board_idx
            ON permission_grants (board_id)
            WHERE board_id IS NOT NULL
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS permission_grants CASCADE")
            .await?;

        for table in &["projects", "folders", "documents", "boards"] {
            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} DROP COLUMN IF EXISTS visibility"
            ))
            .await?;

            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} DROP COLUMN IF EXISTS visibility_role"
            ))
            .await?;
        }

        conn.execute_unprepared("ALTER TABLE users DROP COLUMN IF EXISTS disabled_at")
            .await?;

        // Restore api_keys.role with the original CHECK.
        conn.execute_unprepared(
            r#"ALTER TABLE api_keys ADD COLUMN role TEXT NOT NULL DEFAULT 'agent-standard'
               CONSTRAINT api_keys_role_check CHECK (role IN ('agent-standard'))"#,
        )
        .await?;

        Ok(())
    }
}
