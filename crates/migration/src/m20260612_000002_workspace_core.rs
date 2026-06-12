use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260612_000002_workspace_core"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE property_definitions (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                key TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                options JSONB,
                applies_to TEXT NOT NULL DEFAULT 'task',
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT property_definitions_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX property_definitions_key_uq
               ON property_definitions (workspace_id, key)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE property_definitions
               ADD CONSTRAINT property_definitions_key_format_check
               CHECK (key ~ '^[a-z][a-z0-9_]{0,63}$')"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE property_definitions
               ADD CONSTRAINT property_definitions_kind_check
               CHECK (kind IN ('text','number','boolean','date','select','multi_select'))"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE property_definitions
               ADD CONSTRAINT property_definitions_applies_to_check
               CHECK (applies_to IN ('document','task','both'))"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE projects (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                name TEXT NOT NULL,
                slug TEXT NOT NULL,
                task_prefix TEXT NOT NULL,
                next_task_number INTEGER NOT NULL DEFAULT 0,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT projects_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1),
                CONSTRAINT projects_next_task_number_check
                    CHECK (next_task_number >= 0)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX projects_slug_uq
               ON projects (workspace_id, slug)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX projects_task_prefix_uq
               ON projects (workspace_id, task_prefix)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE projects
               ADD CONSTRAINT projects_task_prefix_format_check
               CHECK (task_prefix ~ '^[A-Z][A-Z0-9]{1,9}$')"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE folders (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                project_id UUID REFERENCES projects(id),
                parent_folder_id UUID REFERENCES folders(id),
                name TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT folders_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX folders_name_uq
               ON folders (workspace_id, project_id, parent_folder_id, name)
               NULLS NOT DISTINCT
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS folders CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS projects CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS property_definitions CASCADE")
            .await?;

        Ok(())
    }
}
