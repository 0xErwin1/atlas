use async_trait::async_trait;
use atlas_domain::{
    DomainError, StatusTemplateId, WorkspaceCtx,
    entities::boards_tasks::PositionBetween,
    entities::status_templates::{NewStatusTemplate, StatusTemplate, StatusTemplatePatch},
    ids::ColumnId,
    position,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, Order, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};

use crate::persistence::entities::status_templates::{status_template, status_template_from};

pub use atlas_domain::ports::status_templates::StatusTemplateRepo;

pub struct PgStatusTemplateRepo {
    pub conn: DatabaseConnection,
}

impl PgStatusTemplateRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl StatusTemplateRepo for PgStatusTemplateRepo {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewStatusTemplate,
    ) -> Result<StatusTemplate, DomainError> {
        let now = Utc::now();
        let model = status_template::ActiveModel {
            id: Set(StatusTemplateId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            name: Set(new.name),
            color: Set(new.color),
            position_key: Set(new.position_key),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(status_template_from)
            .map_err(db_err)
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<StatusTemplate>, DomainError> {
        status_template::Entity::find()
            .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(status_template::Column::DeletedAt.is_null())
            .order_by_asc(status_template::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(status_template_from).collect())
            .map_err(db_err)
    }

    async fn patch(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
        patch: StatusTemplatePatch,
    ) -> Result<StatusTemplate, DomainError> {
        let row = status_template::Entity::find_by_id(id.0)
            .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(status_template::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "status_template",
                id: id.0,
            })?;

        let mut active = row.into_active_model();

        if let Some(name) = patch.name {
            active.name = Set(name);
        }

        if let Some(color) = patch.color {
            active.color = Set(color);
        }

        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(status_template_from)
            .map_err(db_err)
    }

    async fn move_template(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
        position: PositionBetween,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = status_template::Entity::find_by_id(id.0)
            .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(status_template::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "status_template",
                id: id.0,
            })?;

        let new_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    let remap = resequence_templates(&txn, ctx).await?;
                    let rebalanced = remap_anchors(&position, &remap);
                    match position::try_between(
                        rebalanced.before.as_deref(),
                        rebalanced.after.as_deref(),
                    ) {
                        Some(key) => key,
                        None => {
                            txn.rollback().await.map_err(db_err)?;
                            return Err(DomainError::PositionExhausted {
                                column_id: ColumnId(id.0),
                            });
                        }
                    }
                }
            };

        let mut active = row.into_active_model();
        active.position_key = Set(new_key);
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
    ) -> Result<(), DomainError> {
        let row = status_template::Entity::find_by_id(id.0)
            .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(status_template::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "status_template",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

/// Lists all non-deleted templates for a workspace, ordered by position_key.
///
/// Used internally for seeding and apply operations.
pub async fn list_templates_for_workspace(
    conn: &impl ConnectionTrait,
    workspace_id: uuid::Uuid,
) -> Result<Vec<status_template::Model>, DomainError> {
    status_template::Entity::find()
        .filter(status_template::Column::WorkspaceId.eq(workspace_id))
        .filter(status_template::Column::DeletedAt.is_null())
        .order_by_asc(status_template::Column::PositionKey)
        .all(conn)
        .await
        .map_err(db_err)
}

/// Resequences all non-deleted templates in the workspace using evenly spaced
/// fractional keys.
///
/// Selects with FOR UPDATE to serialize concurrent resequencing races.
/// Must run inside an existing transaction.
async fn resequence_templates(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
) -> Result<Vec<(String, String)>, DomainError> {
    let rows = status_template::Entity::find()
        .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(status_template::Column::DeletedAt.is_null())
        .order_by(status_template::Column::PositionKey, Order::Asc)
        .order_by(status_template::Column::Id, Order::Asc)
        .lock_exclusive()
        .all(txn)
        .await
        .map_err(db_err)?;

    let mut remap = Vec::with_capacity(rows.len());
    let mut prev: Option<String> = None;

    for row in rows {
        let old_key = row.position_key.clone();
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(txn).await.map_err(db_err)?;
        remap.push((old_key, key.clone()));
        prev = Some(key);
    }

    Ok(remap)
}

/// Translates stale anchor keys to their post-resequence equivalents.
///
/// Mirrors the `remap_anchors` helper in `boards_tasks.rs`.
fn remap_anchors(original: &PositionBetween, remap: &[(String, String)]) -> PositionBetween {
    let lookup = |old: &str| -> Vec<&String> {
        remap
            .iter()
            .filter(|(o, _)| o == old)
            .map(|(_, n)| n)
            .collect()
    };

    match (&original.before, &original.after) {
        (Some(b), Some(a)) if b == a => {
            let news = lookup(b);
            match (news.first(), news.get(1)) {
                (Some(first), Some(second)) => PositionBetween {
                    before: Some((*first).clone()),
                    after: Some((*second).clone()),
                },
                (Some(only), None) => PositionBetween {
                    before: Some((*only).clone()),
                    after: Some((*only).clone()),
                },
                _ => PositionBetween {
                    before: original.before.clone(),
                    after: original.after.clone(),
                },
            }
        }
        _ => {
            let translate = |anchor: &Option<String>| -> Option<String> {
                anchor
                    .as_ref()
                    .and_then(|key| lookup(key).first().map(|s| (*s).clone()))
            };
            PositionBetween {
                before: translate(&original.before),
                after: translate(&original.after),
            }
        }
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
