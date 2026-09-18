//! E4-S2b: rewrites every `custos.api_keys.scopes` entry from the legacy
//! `<family>:<action>` spelling to the canonical
//! `<product>::<kind>::<action>` wire form, closing S2a's compatibility
//! window: S2a made the reader accept both spellings while writing only the
//! canonical one, so no stored row had to be rewritten on day one; this
//! migration performs that rewrite and the reader's legacy tolerance becomes
//! dead-letter tolerance only.
//!
//! The transformation is applied row by row instead of as one set-based SQL
//! statement. The closed vocabulary (37 catalog entries) must stay exactly
//! `Capability::ALL`: a SQL `regexp_replace`/whitelist copy of that
//! vocabulary would be a second source of truth that can drift from the
//! codec, and the fail-closed rule (unknown or malformed entries are
//! DROPPED, never coerced and never defaulted) is already implemented and
//! pinned in `capabilities_from_stored`. Reusing that reader makes the
//! migration byte-for-byte equivalent to "what the stored reader already
//! accepts", per row, preserving entry order. Idempotent by construction:
//! canonical entries parse to themselves and pass through untouched, so a
//! second application is a no-op.
//!
//! Dropped entries are reported, not silent: each dropped entry is logged by
//! the reader (`authz.scope_drop`), and a per-migration summary counts the
//! total loss so an operator can see it.
//!
//! Runs after `m20260918_000054_custos_principals_not_null`.
//! `crates/migration` stays byte-frozen (INV-MIGRATION-SCOPE): this
//! migration is spliced into `ComposedMigrator`/`ComposedTestMigrator` by
//! `custos_new()`.

use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use sea_orm_migration::prelude::*;

use crate::entities::identity::capabilities_from_stored;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260918_000055_custos_scope_wire_form"
    }
}

#[derive(FromQueryResult)]
struct KeyScopes {
    id: uuid::Uuid,
    scopes: Vec<String>,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        let keys = KeyScopes::find_by_statement(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, scopes FROM custos.api_keys".to_owned(),
        ))
        .all(conn)
        .await?;

        let mut dropped_total: usize = 0;
        for key in &keys {
            let kept = capabilities_from_stored(&key.scopes);
            dropped_total += key.scopes.len() - kept.len();

            let rewritten: Vec<String> = kept.iter().map(|cap| cap.as_str().to_owned()).collect();
            if rewritten != key.scopes {
                conn.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE custos.api_keys SET scopes = $1 WHERE id = $2",
                    [rewritten.into(), key.id.into()],
                ))
                .await?;
            }
        }

        if dropped_total > 0 {
            tracing::warn!(
                target: "authz.scope_drop",
                event = "migration_scope_drop",
                dropped_entries = dropped_total,
                "m20260918_000055: dropped unparseable stored scope entries while rewriting \
                 api key scopes to the canonical wire form; those entries granted nothing \
                 before the migration and grant nothing after it"
            );
        }

        Ok(())
    }

    /// Restores the legacy `<family>:<action>` spelling by stripping the
    /// product segment. Lossy by design: entries `up()` dropped (unknown or
    /// malformed strings) have no pre-rewrite bytes to restore, so they are
    /// passed through untouched and keep being dropped by the stored reader.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        let keys = KeyScopes::find_by_statement(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, scopes FROM custos.api_keys".to_owned(),
        ))
        .all(conn)
        .await?;

        for key in &keys {
            let mut restored = Vec::with_capacity(key.scopes.len());
            for entry in &key.scopes {
                match canonical_to_legacy(entry) {
                    Some(legacy) => restored.push(legacy),
                    None => restored.push(entry.clone()),
                }
            }

            if restored != key.scopes {
                conn.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE custos.api_keys SET scopes = $1 WHERE id = $2",
                    [restored.into(), key.id.into()],
                ))
                .await?;
            }
        }

        Ok(())
    }
}

/// Inverts the canonical `<product>::<kind>::<action>` spelling for entries
/// that carry a known product segment; anything else has no legacy form.
fn canonical_to_legacy(entry: &str) -> Option<String> {
    let (product, rest) = entry.split_once("::")?;
    match product {
        "acta" | "custos" => Some(rest.replace("::", ":")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260918_000055_custos_scope_wire_form");
    }
}
