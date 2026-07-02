use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260702_000035_automation_add_comment_action"
    }
}

/// Widens the `automation_rules.action_type` CHECK whitelist to accept
/// `add_comment` alongside `create_task`, enabling the E11 rule engine to post a
/// comment on a task in response to an external event (the original motivation
/// for the comments primitive).
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE automation_rules
                DROP CONSTRAINT IF EXISTS automation_rules_action_type_check,
                ADD CONSTRAINT automation_rules_action_type_check CHECK (
                    action_type IN ('create_task', 'add_comment')
                )
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE automation_rules
                DROP CONSTRAINT IF EXISTS automation_rules_action_type_check,
                ADD CONSTRAINT automation_rules_action_type_check CHECK (
                    action_type IN ('create_task')
                )
            "#,
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(
            Migration.name(),
            "m20260702_000035_automation_add_comment_action"
        );
    }
}
