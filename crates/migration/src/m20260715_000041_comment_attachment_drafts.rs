use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260715_000041_comment_attachment_drafts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE comment_attachment_drafts (
                    id UUID PRIMARY KEY,
                    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE RESTRICT,
                    task_id UUID REFERENCES tasks(id) ON DELETE RESTRICT,
                    document_id UUID REFERENCES documents(id) ON DELETE RESTRICT,
                    created_by_user_id UUID REFERENCES users(id) ON DELETE RESTRICT,
                    created_by_api_key_id UUID REFERENCES api_keys(id) ON DELETE RESTRICT,
                    create_token TEXT NOT NULL,
                    create_digest BYTEA NOT NULL CHECK (octet_length(create_digest) = 32),
                    state TEXT NOT NULL CHECK (state IN ('active', 'finalized', 'cancelled', 'expired', 'deleted_finalized')),
                    finalized_comment_id UUID REFERENCES comments(id) ON DELETE RESTRICT,
                    final_body_digest BYTEA CHECK (final_body_digest IS NULL OR octet_length(final_body_digest) = 32),
                    final_request_digest BYTEA CHECK (final_request_digest IS NULL OR octet_length(final_request_digest) = 32),
                    expires_at TIMESTAMPTZ NOT NULL,
                    terminal_at TIMESTAMPTZ,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    CONSTRAINT comment_attachment_drafts_parent_check
                        CHECK (num_nonnulls(task_id, document_id) = 1),
                    CONSTRAINT comment_attachment_drafts_principal_check
                        CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
                );
                CREATE UNIQUE INDEX comment_attachment_drafts_user_create_token_idx
                    ON comment_attachment_drafts (workspace_id, created_by_user_id, create_token)
                    WHERE created_by_user_id IS NOT NULL;
                CREATE UNIQUE INDEX comment_attachment_drafts_api_key_create_token_idx
                    ON comment_attachment_drafts (workspace_id, created_by_api_key_id, create_token)
                    WHERE created_by_api_key_id IS NOT NULL;
                CREATE INDEX comment_attachment_drafts_expiry_idx
                    ON comment_attachment_drafts (state, expires_at);
                CREATE INDEX comment_attachment_drafts_terminal_idx
                    ON comment_attachment_drafts (terminal_at)
                    WHERE terminal_at IS NOT NULL;

                ALTER TABLE attachments ADD COLUMN draft_id UUID
                    REFERENCES comment_attachment_drafts(id) ON DELETE RESTRICT;
                ALTER TABLE attachments DROP CONSTRAINT attachments_owner_check;
                ALTER TABLE attachments ADD CONSTRAINT attachments_owner_check
                    CHECK (num_nonnulls(document_id, task_id, comment_id, draft_id) = 1);
                CREATE INDEX attachments_draft_owner_idx
                    ON attachments (workspace_id, draft_id)
                    WHERE draft_id IS NOT NULL;

                CREATE TABLE comment_attachment_draft_uploads (
                    draft_id UUID NOT NULL REFERENCES comment_attachment_drafts(id) ON DELETE RESTRICT,
                    upload_token TEXT NOT NULL,
                    original_attachment_id UUID NOT NULL UNIQUE REFERENCES attachments(id) ON DELETE RESTRICT,
                    attachment_id UUID UNIQUE REFERENCES attachments(id) ON DELETE RESTRICT,
                    request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
                    payload_digest BYTEA NOT NULL CHECK (octet_length(payload_digest) = 32),
                    file_name TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    size_bytes BIGINT NOT NULL,
                    deleted_at TIMESTAMPTZ,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    CONSTRAINT comment_attachment_draft_uploads_live_attachment_check
                        CHECK (attachment_id IS NOT NULL OR deleted_at IS NOT NULL),
                    PRIMARY KEY (draft_id, upload_token)
                );
                CREATE INDEX comment_attachment_draft_uploads_live_idx
                    ON comment_attachment_draft_uploads (draft_id, created_at)
                    WHERE deleted_at IS NULL;
                CREATE INDEX comment_attachment_draft_uploads_original_attachment_idx
                    ON comment_attachment_draft_uploads (draft_id, original_attachment_id);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DO $$
                BEGIN
                    IF EXISTS (SELECT 1 FROM attachments WHERE draft_id IS NOT NULL)
                       OR EXISTS (SELECT 1 FROM comment_attachment_drafts)
                       OR EXISTS (SELECT 1 FROM comment_attachment_draft_uploads) THEN
                        RAISE EXCEPTION 'cannot roll back comment attachment drafts while draft records exist';
                    END IF;
                END $$;

                DROP TABLE comment_attachment_draft_uploads;
                ALTER TABLE attachments DROP CONSTRAINT attachments_owner_check;
                DROP INDEX attachments_draft_owner_idx;
                ALTER TABLE attachments DROP COLUMN draft_id;
                ALTER TABLE attachments ADD CONSTRAINT attachments_owner_check
                    CHECK (num_nonnulls(document_id, task_id, comment_id) = 1);
                DROP TABLE comment_attachment_drafts;
                "#,
            )
            .await
            .map(|_| ())
    }
}
