//! E4-S3a: gives every `agent` principal a first-class owner relationship —
//! `custos.principals.owner_user_id`, a nullable FK to `custos.users(id)` —
//! and enforces the invariant with a table CHECK:
//!
//! `(kind = 'agent') = (owner_user_id IS NOT NULL)`
//!
//! An agent must have exactly one owning human user; a user principal must
//! have none (a user's principal identity is the user row itself, and a
//! self-owned user would make the owner relation meaningless). Rejected
//! alternatives: an FK to `custos.principals` (cannot guarantee the owner is
//! human) and a separate `agent_owners` table (a second table for one
//! column of state the mirror already owns).
//!
//! The back-fill takes each agent principal's owner from its key's
//! `created_by_user_id` (`custos.api_keys.principal_id →
//! api_keys.created_by_user_id`): before this migration every writer minted
//! an agent principal exactly once, alongside exactly one key, and recorded
//! the human creator only on that key row. The migration therefore must run
//! the back-fill before applying the CHECK — the constraint must pass on a
//! real, pre-existing database, not only on a fresh one. One migration
//! covers the whole shape change so the framework's per-migration
//! transaction makes add-back-fill-constrain atomic.
//!
//! `down()` drops the CHECK, then the column. `crates/migration` stays
//! byte-frozen (INV-MIGRATION-SCOPE): this migration is spliced into
//! `ComposedMigrator`/`ComposedTestMigrator` by `custos_new()`.

use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260919_000056_custos_principal_owner"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "ALTER TABLE custos.principals \
             ADD COLUMN owner_user_id UUID NULL REFERENCES custos.users(id)",
        )
        .await?;

        // Back-fill before constraining: every agent principal to date was
        // minted alongside exactly one api key, and that key row is
        // the only place the human creator was recorded.
        conn.execute_unprepared(
            "UPDATE custos.principals p \
             SET owner_user_id = k.created_by_user_id \
             FROM custos.api_keys k \
             WHERE k.principal_id = p.id AND p.kind = 'agent'",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE custos.principals \
             ADD CONSTRAINT custos_principals_kind_owner_check \
             CHECK ((kind = 'agent') = (owner_user_id IS NOT NULL))",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "ALTER TABLE custos.principals \
             DROP CONSTRAINT custos_principals_kind_owner_check",
        )
        .await?;
        conn.execute_unprepared("ALTER TABLE custos.principals DROP COLUMN owner_user_id")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260919_000056_custos_principal_owner");
    }
}
