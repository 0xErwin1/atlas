//! Repository implementation for the `platform_status_templates` port,
//! moved from `atlas_server::persistence::repos::platform_status_templates`
//! (orchestrator-mandated addition to S4 PR9).
//!
//! `platform_status_templates` has no `workspace_id` column and no foreign
//! key at all — despite the name, it is Acta-owned seed data (design §D1's
//! R8 classification), not platform runtime state, and has no Custos
//! dependency. Reuses `status_templates::remap_anchors`, a
//! `pub(crate)` helper now co-located in this crate after the move.

use async_trait::async_trait;
use atlas_acta::entities::boards_tasks::PositionBetween;
use atlas_acta::entities::status_templates::NewStatusTemplate;
use atlas_acta::entities::status_templates::PlatformStatusTemplate;
use atlas_acta::entities::status_templates::StatusTemplatePatch;
use atlas_acta::ids::PlatformStatusTemplateId;
use atlas_core::error::DomainError;
use atlas_core::position;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, Order, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};

use crate::entities::status_templates::{platform_status_template, platform_status_template_from};
use crate::repos::status_templates::remap_anchors;
use atlas_postgres::db_err;

pub use atlas_acta::ports::status_templates::PlatformStatusTemplateRepo;

// `atlas_server::error::acta_conflict::POSITION_EXHAUSTED` cannot be
// imported across the crate boundary (this crate must not depend on
// `atlas_server`). Duplicated as a local, byte-identical constant, mirroring
// `status_templates.rs`'s own copy in this crate.
mod acta_conflict {
    pub(super) const POSITION_EXHAUSTED: &str = "position-exhausted";
}

pub struct PgPlatformStatusTemplateRepo {
    pub conn: DatabaseConnection,
}

impl PgPlatformStatusTemplateRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl PlatformStatusTemplateRepo for PgPlatformStatusTemplateRepo {
    async fn create(&self, new: NewStatusTemplate) -> Result<PlatformStatusTemplate, DomainError> {
        let now = Utc::now();
        let model = platform_status_template::ActiveModel {
            id: Set(PlatformStatusTemplateId::new().0),
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
            .map(platform_status_template_from)
            .map_err(db_err)
    }

    async fn list(&self) -> Result<Vec<PlatformStatusTemplate>, DomainError> {
        platform_status_template::Entity::find()
            .filter(platform_status_template::Column::DeletedAt.is_null())
            .order_by_asc(platform_status_template::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(platform_status_template_from)
                    .collect()
            })
            .map_err(db_err)
    }

    async fn patch(
        &self,
        id: PlatformStatusTemplateId,
        patch: StatusTemplatePatch,
    ) -> Result<PlatformStatusTemplate, DomainError> {
        let row = find_live(&self.conn, id).await?;

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
            .map(platform_status_template_from)
            .map_err(db_err)
    }

    async fn move_template(
        &self,
        id: PlatformStatusTemplateId,
        position: PositionBetween,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = find_live(&txn, id).await?;

        let new_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    let remap = resequence_templates(&txn).await?;
                    let rebalanced = remap_anchors(&position, &remap);
                    match position::try_between(
                        rebalanced.before.as_deref(),
                        rebalanced.after.as_deref(),
                    ) {
                        Some(key) => key,
                        None => {
                            txn.rollback().await.map_err(db_err)?;
                            return Err(DomainError::ComponentConflict {
                                code: acta_conflict::POSITION_EXHAUSTED,
                                message: None,
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

    async fn soft_delete(&self, id: PlatformStatusTemplateId) -> Result<(), DomainError> {
        let row = find_live(&self.conn, id).await?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

async fn find_live(
    conn: &impl ConnectionTrait,
    id: PlatformStatusTemplateId,
) -> Result<platform_status_template::Model, DomainError> {
    platform_status_template::Entity::find_by_id(id.0)
        .filter(platform_status_template::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "platform_status_template",
            id: id.0,
        })
}

/// Resequences all non-deleted platform templates using evenly spaced fractional
/// keys, mirroring the workspace-level repo.
///
/// Selects with FOR UPDATE to serialize concurrent resequencing races.
/// Must run inside an existing transaction.
async fn resequence_templates(
    txn: &impl ConnectionTrait,
) -> Result<Vec<(String, String)>, DomainError> {
    let rows = platform_status_template::Entity::find()
        .filter(platform_status_template::Column::DeletedAt.is_null())
        .order_by(platform_status_template::Column::PositionKey, Order::Asc)
        .order_by(platform_status_template::Column::Id, Order::Asc)
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
