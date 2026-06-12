use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260612_000001_identity"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE workspaces (
                id UUID PRIMARY KEY,
                name TEXT NOT NULL,
                slug TEXT NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE workspaces ADD CONSTRAINT workspaces_slug_format_check
               CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$')"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE users (
                id UUID PRIMARY KEY,
                username TEXT NOT NULL,
                display_name TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                is_root BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX users_username_lower_uq ON users (lower(username))"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX users_single_root_uq ON users ((true)) WHERE is_root"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE sessions (
                id UUID PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL UNIQUE,
                expires_at TIMESTAMPTZ NOT NULL,
                last_used_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(r#"CREATE INDEX sessions_user_id_idx ON sessions (user_id)"#)
            .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE api_keys (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                created_by_user_id UUID NOT NULL REFERENCES users(id),
                name TEXT NOT NULL,
                token_hash TEXT NOT NULL UNIQUE,
                role TEXT NOT NULL DEFAULT 'agent-standard',
                expires_at TIMESTAMPTZ,
                last_used_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT api_keys_role_check CHECK (role IN ('agent-standard'))
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX api_keys_workspace_idx ON api_keys (workspace_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE workspace_memberships (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                user_id UUID NOT NULL REFERENCES users(id),
                role TEXT NOT NULL DEFAULT 'member',
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                UNIQUE (workspace_id, user_id),
                CONSTRAINT membership_role_check CHECK (role IN ('owner', 'admin', 'member'))
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX memberships_user_id_idx ON workspace_memberships (user_id)"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS workspace_memberships CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS api_keys CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS sessions CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS users CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS workspaces CASCADE")
            .await?;

        Ok(())
    }
}
