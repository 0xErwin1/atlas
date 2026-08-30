//! S3d: moves the eight Custos-owned tables into the `custos` schema via
//! `ALTER TABLE ... SET SCHEMA`, one statement per table:
//! `users`, `sessions`, `user_activation_tokens`, `api_keys`, `groups`,
//! `group_members`, `permission_grants`, `security_audit_log`.
//!
//! Deliberately excluded: `user_ui_state` (platform), `workspaces` and
//! `workspace_memberships` (Acta, S4), `purge_operations` (Acta lifecycle).
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, constraint, and inbound foreign key survives untouched — Postgres
//! binds a foreign key to the referenced table's OID, not its qualified
//! name, so the many Acta-owned tables that reference `users`/`api_keys`/
//! `groups` keep resolving to the moved rows with no DDL of their own. No
//! `search_path` change accompanies this migration (design §S3d): every
//! caller must qualify its own SQL with `custos.`, which is the point of the
//! CI grep gate this PR also adds.
//!
//! Deployment contract: like `m20260830_000050_grant_resource_ref`, this is
//! a single stop-the-world migration. An old binary compiled against
//! unqualified `public.*` table names would fail every Custos-touching query
//! the moment this migration commits. Atlas deploys as a single instance
//! whose binary starts only after its migrator has run, so no old/new
//! binary ever runs concurrently against a mixed schema; a rollback must
//! pair with this migration's `down()`, which moves the eight tables back to
//! `public` unchanged (`SET SCHEMA public`, clean — no data transformation,
//! unlike the O1 migration's down path).

use sea_orm_migration::prelude::*;

const CUSTOS_TABLES: &[&str] = &[
    "users",
    "sessions",
    "user_activation_tokens",
    "api_keys",
    "groups",
    "group_members",
    "permission_grants",
    "security_audit_log",
];

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260830_000051_custos_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("CREATE SCHEMA IF NOT EXISTS custos")
            .await?;

        for table in CUSTOS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA custos"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in CUSTOS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE custos.{table} SET SCHEMA public"))
                .await?;
        }

        conn.execute_unprepared("DROP SCHEMA IF EXISTS custos")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260830_000051_custos_set_schema");
    }
}
