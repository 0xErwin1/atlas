use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260702_000037_prune_description_edit_activity"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Prunes historical `field_changed` activity rows for task descriptions.
    ///
    /// A description autosaves in bursts and previously produced one activity entry
    /// per save, flooding the feed with uninformative "changed a field" rows.
    /// Description edits are no longer recorded going forward; this removes the
    /// accumulated noise. Every other activity kind — creates, moves, assignments,
    /// and non-description field changes — is left untouched.
    ///
    /// The payload is the externally-tagged `ActivityPayload::FieldChanged`, so the
    /// changed field lives at `payload -> 'field_changed' ->> 'field'`.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DELETE FROM task_activity
                WHERE kind = 'field_changed'
                  AND payload -> 'field_changed' ->> 'field' = 'description'
                "#,
            )
            .await?;

        Ok(())
    }

    /// Irreversible: the pruned rows carried no recoverable information.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
