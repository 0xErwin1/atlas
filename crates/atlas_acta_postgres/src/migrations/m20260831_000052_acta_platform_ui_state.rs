//! S4 PR9: moves `user_ui_state` into a dedicated `platform` schema and
//! renames it to `ui_state`.
//!
//! `user_ui_state` is neither Custos (identity/security) nor Acta
//! (workspace content) — it is per-user client preference state the server
//! owns directly (design §D4). It is the first migration `acta_new()`
//! contributes, ordered before every `SET SCHEMA acta` batch because it has
//! no dependency on the D1 table classification work and de-risks the
//! smallest, least-ambiguous move first.
//!
//! `SET SCHEMA` does not rename a table, so a second statement renames it
//! inside its new schema. This is safe under the route/DTO freeze because
//! `user_ui_state` has no route-facing name today (routes speak DTOs, not
//! table names); nothing observable changes. Keyed by `user_id`, unchanged —
//! `custos.principals` does not exist before E4, so there is no re-key
//! target (spec requirement 4).
//!
//! Deployment contract: like `m20260830_000051_custos_set_schema`, this is a
//! single stop-the-world migration. Atlas deploys as a single instance whose
//! binary starts only after its migrator has run, so no old/new binary ever
//! runs concurrently against a mixed schema; a rollback must pair with this
//! migration's `down()`, which reverses both statements in the opposite
//! order and is clean — `SET SCHEMA`/`RENAME TO` move and rename a table
//! without recreating it, so every index, check and foreign key survives
//! unchanged (Postgres binds a foreign key to the referenced table's OID,
//! not its qualified name), no data transformation either direction.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260831_000052_acta_platform_ui_state"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("CREATE SCHEMA IF NOT EXISTS platform")
            .await?;
        conn.execute_unprepared("ALTER TABLE user_ui_state SET SCHEMA platform")
            .await?;
        conn.execute_unprepared("ALTER TABLE platform.user_ui_state RENAME TO ui_state")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE platform.ui_state RENAME TO user_ui_state")
            .await?;
        conn.execute_unprepared("ALTER TABLE platform.user_ui_state SET SCHEMA public")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260831_000052_acta_platform_ui_state");
    }
}
