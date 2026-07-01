use atlas_domain::DomainError;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement,
};
use uuid::Uuid;

use crate::persistence::entities::automation_rule::automation_rules;

/// Fields that may be updated on an existing automation rule. `None` leaves the
/// corresponding field unchanged; `Some(None)` clears a nullable field such as
/// `trigger_filter`.
pub struct AutomationRulePatch {
    pub name: Option<String>,
    pub is_active: Option<bool>,
    pub trigger_filter: Option<Option<serde_json::Value>>,
    pub action_params: Option<serde_json::Value>,
}

pub struct PgAutomationRuleRepo;

impl PgAutomationRuleRepo {
    /// Inserts a new automation rule.
    ///
    /// Returns `DomainError::InvalidInput` when `trigger_event_type` does not
    /// start with `"external."`. This mirrors the DB-level CHECK constraint and
    /// ensures the cascade guard is enforced at the application layer too.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        name: String,
        trigger_event_type: String,
        trigger_filter: Option<serde_json::Value>,
        project_id: Option<Uuid>,
        action_type: String,
        action_params: serde_json::Value,
        created_by_user_id: Uuid,
    ) -> Result<automation_rules::Model, DomainError> {
        if !trigger_event_type.starts_with("external.") {
            return Err(DomainError::InvalidInput {
                message: format!(
                    "trigger_event_type must match 'external.*', got: {trigger_event_type}"
                ),
            });
        }

        if project_id.is_some() {
            return Err(DomainError::InvalidInput {
                message:
                    "external automation rules must be workspace-scoped in v1; omit project_id"
                        .into(),
            });
        }

        let now = Utc::now();
        let model = automation_rules::ActiveModel {
            id: Set(Uuid::now_v7()),
            workspace_id: Set(workspace_id),
            name: Set(name),
            is_active: Set(true),
            trigger_event_type: Set(trigger_event_type),
            trigger_filter: Set(trigger_filter),
            project_id: Set(project_id),
            action_type: Set(action_type),
            action_params: Set(action_params),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };

        model.insert(conn).await.map_err(db_err)
    }

    /// Lists all non-deleted rules for a workspace ordered by creation time,
    /// with cursor-based pagination. `after_id` is the last seen rule ID.
    pub async fn list(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<automation_rules::Model>, DomainError> {
        let mut q = automation_rules::Entity::find()
            .filter(automation_rules::Column::WorkspaceId.eq(workspace_id))
            .filter(automation_rules::Column::DeletedAt.is_null())
            .order_by_asc(automation_rules::Column::Id);

        if let Some(after) = after_id {
            q = q.filter(automation_rules::Column::Id.gt(after));
        }

        q.limit(limit).all(conn).await.map_err(db_err)
    }

    /// Returns a single non-deleted rule by its UUID within a workspace.
    pub async fn get(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Option<automation_rules::Model>, DomainError> {
        automation_rules::Entity::find_by_id(id)
            .filter(automation_rules::Column::WorkspaceId.eq(workspace_id))
            .filter(automation_rules::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)
    }

    /// Returns all active, non-deleted rules in `workspace_id` whose
    /// `trigger_event_type` matches the given string.
    ///
    /// Scope matching: a rule with `project_id IS NULL` is workspace-scoped and
    /// always matches; a rule with `project_id = P` matches only when
    /// `event_project_id = Some(P)`. If `event_project_id` is `None`, only
    /// workspace-scoped rules match.
    pub async fn list_active_for_workspace_event(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        trigger_event_type: &str,
        event_project_id: Option<Uuid>,
    ) -> Result<Vec<automation_rules::Model>, DomainError> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT id, workspace_id, name, is_active, trigger_event_type, trigger_filter,
                   project_id, action_type, action_params, created_by_user_id,
                   created_at, updated_at, deleted_at
            FROM   automation_rules
            WHERE  workspace_id        = $1
              AND  trigger_event_type  = $2
              AND  is_active           = true
              AND  deleted_at          IS NULL
              AND  (project_id IS NULL OR project_id = $3)
            ORDER  BY created_at
            "#,
            [
                workspace_id.into(),
                trigger_event_type.into(),
                event_project_id.into(),
            ],
        );

        automation_rules::Entity::find()
            .from_raw_sql(stmt)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Applies a partial update to a rule. Fields absent from `patch` are left
    /// unchanged.
    pub async fn patch(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
        patch: AutomationRulePatch,
    ) -> Result<automation_rules::Model, DomainError> {
        let row = automation_rules::Entity::find_by_id(id)
            .filter(automation_rules::Column::WorkspaceId.eq(workspace_id))
            .filter(automation_rules::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "automation_rule",
                id,
            })?;

        let mut active = row.into_active_model();

        if let Some(name) = patch.name {
            active.name = Set(name);
        }

        if let Some(is_active) = patch.is_active {
            active.is_active = Set(is_active);
        }

        if let Some(trigger_filter) = patch.trigger_filter {
            active.trigger_filter = Set(trigger_filter);
        }

        if let Some(action_params) = patch.action_params {
            active.action_params = Set(action_params);
        }

        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)
    }

    /// Soft-deletes a rule by setting `deleted_at`.
    pub async fn soft_delete(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let row = automation_rules::Entity::find_by_id(id)
            .filter(automation_rules::Column::WorkspaceId.eq(workspace_id))
            .filter(automation_rules::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "automation_rule",
                id,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;

        Ok(())
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
