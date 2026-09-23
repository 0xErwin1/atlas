//! E5-S3 (`v2-e5-s3-authz-storage`): the V2 authorization tables —
//! `custos.roles`, `custos.grants_v2` and `custos.deny_rules`. Purely
//! additive: the V1 `custos.permission_grants` table and its resolver stay
//! untouched until E7/MIG.
//!
//! Every row addresses exactly one subject (`subject_kind` plus the single
//! matching column: `principal`, `group` or `principal_set`) and one target,
//! stored as a typed string (`target_kind` in `ref|path|selector` plus the
//! canonical text form), never as a foreign key: targets may name resources
//! Custos does not own. A grant carries exactly one authority
//! (`authority_kind`): a built-in role `name@version` from a product
//! catalog, a custom role row in `custos.roles` (`ON DELETE RESTRICT`, so a
//! referenced role cannot disappear under its grants), or an explicit
//! non-empty action list. Deny rules always carry an explicit non-empty
//! action list. Built-in role definitions are not stored here; only the
//! `name@version` a grant references is.
//!
//! `crates/migration` stays byte-frozen (INV-MIGRATION-SCOPE): this
//! migration is spliced into `ComposedMigrator`/`ComposedTestMigrator` by
//! `custos_new()`.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260923_000058_custos_v2_authorization"
    }
}

const SUBJECT_COLUMNS: &str = "\
    subject_kind          TEXT NOT NULL \
        CHECK (subject_kind IN ('principal','group','principal_set')), \
    subject_principal_id  UUID NULL, \
    subject_group_id      UUID NULL, \
    subject_principal_set TEXT NULL";

const TARGET_COLUMNS: &str = "\
    target_kind TEXT NOT NULL CHECK (target_kind IN ('ref','path','selector')), \
    target      TEXT NOT NULL, \
    product     TEXT NOT NULL";

/// Exactly one subject column is set, and it is the one `subject_kind` names.
fn subject_check(constraint: &str) -> String {
    format!(
        "CONSTRAINT {constraint} CHECK ( \
             (subject_kind = 'principal' \
                 AND subject_principal_id IS NOT NULL \
                 AND subject_group_id IS NULL \
                 AND subject_principal_set IS NULL) \
             OR (subject_kind = 'group' \
                 AND subject_group_id IS NOT NULL \
                 AND subject_principal_id IS NULL \
                 AND subject_principal_set IS NULL) \
             OR (subject_kind = 'principal_set' \
                 AND subject_principal_set IS NOT NULL \
                 AND subject_principal_id IS NULL \
                 AND subject_group_id IS NULL) \
         )"
    )
}

fn subject_and_target_indexes(table: &str) -> [String; 4] {
    [
        format!(
            "CREATE INDEX custos_{table}_product_target_idx \
             ON custos.{table} (product, target)"
        ),
        format!(
            "CREATE INDEX custos_{table}_subject_principal_idx \
             ON custos.{table} (subject_principal_id) \
             WHERE subject_principal_id IS NOT NULL"
        ),
        format!(
            "CREATE INDEX custos_{table}_subject_group_idx \
             ON custos.{table} (subject_group_id) \
             WHERE subject_group_id IS NOT NULL"
        ),
        format!(
            "CREATE INDEX custos_{table}_subject_principal_set_idx \
             ON custos.{table} (subject_principal_set) \
             WHERE subject_principal_set IS NOT NULL"
        ),
    ]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "CREATE TABLE custos.roles ( \
                 id         UUID PRIMARY KEY, \
                 product    TEXT NOT NULL, \
                 name       TEXT NOT NULL, \
                 actions    TEXT[] NOT NULL, \
                 created_by UUID NOT NULL, \
                 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), \
                 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(), \
                 CONSTRAINT custos_roles_actions_check CHECK (cardinality(actions) > 0), \
                 CONSTRAINT custos_roles_product_name_key UNIQUE (product, name) \
             )",
        )
        .await?;

        conn.execute_unprepared(&format!(
            "CREATE TABLE custos.grants_v2 ( \
                 id             UUID PRIMARY KEY, \
                 {SUBJECT_COLUMNS}, \
                 {TARGET_COLUMNS}, \
                 authority_kind TEXT NOT NULL \
                     CHECK (authority_kind IN ('builtin','custom','actions')), \
                 role_name      TEXT NULL, \
                 role_version   INTEGER NULL, \
                 role_id        UUID NULL \
                     CONSTRAINT custos_grants_v2_role_id_fkey \
                     REFERENCES custos.roles (id) ON DELETE RESTRICT, \
                 actions        TEXT[] NULL, \
                 created_by     UUID NOT NULL, \
                 created_at     TIMESTAMPTZ NOT NULL DEFAULT now(), \
                 {}, \
                 CONSTRAINT custos_grants_v2_authority_check CHECK ( \
                     (authority_kind = 'builtin' \
                         AND role_name IS NOT NULL AND role_version IS NOT NULL \
                         AND role_id IS NULL AND actions IS NULL) \
                     OR (authority_kind = 'custom' \
                         AND role_id IS NOT NULL \
                         AND role_name IS NULL AND role_version IS NULL AND actions IS NULL) \
                     OR (authority_kind = 'actions' \
                         AND actions IS NOT NULL AND cardinality(actions) > 0 \
                         AND role_name IS NULL AND role_version IS NULL AND role_id IS NULL) \
                 ) \
             )",
            subject_check("custos_grants_v2_subject_check")
        ))
        .await?;

        conn.execute_unprepared(&format!(
            "CREATE TABLE custos.deny_rules ( \
                 id         UUID PRIMARY KEY, \
                 {SUBJECT_COLUMNS}, \
                 {TARGET_COLUMNS}, \
                 actions    TEXT[] NOT NULL, \
                 created_by UUID NOT NULL, \
                 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), \
                 {}, \
                 CONSTRAINT custos_deny_rules_actions_check CHECK (cardinality(actions) > 0) \
             )",
            subject_check("custos_deny_rules_subject_check")
        ))
        .await?;

        for table in ["grants_v2", "deny_rules"] {
            for statement in subject_and_target_indexes(table) {
                conn.execute_unprepared(&statement).await?;
            }
        }

        conn.execute_unprepared(
            "CREATE INDEX custos_grants_v2_role_id_idx \
             ON custos.grants_v2 (role_id) \
             WHERE role_id IS NOT NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP INDEX custos.custos_grants_v2_role_id_idx")
            .await?;
        conn.execute_unprepared("DROP TABLE custos.deny_rules")
            .await?;
        conn.execute_unprepared("DROP TABLE custos.grants_v2")
            .await?;
        conn.execute_unprepared("DROP TABLE custos.roles").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260923_000058_custos_v2_authorization");
    }
}
