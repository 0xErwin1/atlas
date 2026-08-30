use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::principal::GroupId;
use atlas_core::principal::UserId;
use atlas_custos::WorkspaceScope;
use atlas_custos::entities::groups::Group;
use atlas_custos::entities::groups::GroupMember;
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::ports::group_repo::GroupRepo as GroupRepoTrait;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder,
};

use crate::entities::permissions::{group, group_member};
use atlas_postgres::db_err;

pub struct PgGroupRepo {
    pub conn: DatabaseConnection,
}

impl PgGroupRepo {
    pub async fn create_in<C: ConnectionTrait>(
        conn: &C,
        new_group: NewGroup,
    ) -> Result<Group, DomainError> {
        let id = GroupId::new();
        let now = Utc::now();

        let model = group::ActiveModel {
            id: Set(id.0),
            workspace_id: Set(new_group.workspace_id.0),
            name: Set(new_group.name),
            created_by: Set(new_group.created_by.0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };

        let inserted = model.insert(conn).await.map_err(|e| {
            if matches!(
                e.sql_err(),
                Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
            ) {
                DomainError::AlreadyExists {
                    message: "a group with this name already exists in the workspace".into(),
                }
            } else {
                db_err(e)
            }
        })?;

        Ok(group_model_to_domain(inserted))
    }

    pub async fn soft_delete_in<C: ConnectionTrait>(
        conn: &C,
        id: GroupId,
        workspace_id: WorkspaceScope,
    ) -> Result<bool, DomainError> {
        let now = Utc::now();

        let existing = group::Entity::find()
            .filter(group::Column::Id.eq(id.0))
            .filter(group::Column::WorkspaceId.eq(workspace_id.0))
            .filter(group::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?;

        let Some(row) = existing else {
            return Ok(false);
        };

        let mut active: group::ActiveModel = row.into();
        active.deleted_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(conn).await.map_err(db_err)?;

        Ok(true)
    }

    pub async fn add_member_in<C: ConnectionTrait>(
        conn: &C,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<GroupMember, DomainError> {
        let now = Utc::now();

        let model = group_member::ActiveModel {
            group_id: Set(group_id.0),
            user_id: Set(user_id.0),
            created_at: Set(now),
        };

        model.insert(conn).await.map_err(db_err)?;

        Ok(GroupMember {
            group_id,
            user_id,
            created_at: now,
        })
    }

    pub async fn remove_member_in<C: ConnectionTrait>(
        conn: &C,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<bool, DomainError> {
        let result = group_member::Entity::delete_many()
            .filter(group_member::Column::GroupId.eq(group_id.0))
            .filter(group_member::Column::UserId.eq(user_id.0))
            .exec(conn)
            .await
            .map_err(db_err)?;

        Ok(result.rows_affected > 0)
    }
}

fn group_model_to_domain(m: group::Model) -> Group {
    Group {
        id: GroupId(m.id),
        workspace_id: WorkspaceScope(m.workspace_id),
        name: m.name,
        created_by: UserId(m.created_by),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

#[async_trait]
impl GroupRepoTrait for PgGroupRepo {
    async fn create(&self, new_group: NewGroup) -> Result<Group, DomainError> {
        PgGroupRepo::create_in(&self.conn, new_group).await
    }

    async fn get(
        &self,
        id: GroupId,
        workspace_id: WorkspaceScope,
    ) -> Result<Option<Group>, DomainError> {
        let row = group::Entity::find()
            .filter(group::Column::Id.eq(id.0))
            .filter(group::Column::WorkspaceId.eq(workspace_id.0))
            .filter(group::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(row.map(group_model_to_domain))
    }

    async fn list(&self, workspace_id: WorkspaceScope) -> Result<Vec<Group>, DomainError> {
        let rows = group::Entity::find()
            .filter(group::Column::WorkspaceId.eq(workspace_id.0))
            .filter(group::Column::DeletedAt.is_null())
            .order_by_asc(group::Column::Name)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(rows.into_iter().map(group_model_to_domain).collect())
    }

    async fn soft_delete(
        &self,
        id: GroupId,
        workspace_id: WorkspaceScope,
    ) -> Result<bool, DomainError> {
        PgGroupRepo::soft_delete_in(&self.conn, id, workspace_id).await
    }

    async fn add_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<GroupMember, DomainError> {
        PgGroupRepo::add_member_in(&self.conn, group_id, user_id).await
    }

    async fn remove_member(&self, group_id: GroupId, user_id: UserId) -> Result<bool, DomainError> {
        PgGroupRepo::remove_member_in(&self.conn, group_id, user_id).await
    }

    async fn list_members(&self, group_id: GroupId) -> Result<Vec<GroupMember>, DomainError> {
        let rows = group_member::Entity::find()
            .filter(group_member::Column::GroupId.eq(group_id.0))
            .order_by_asc(group_member::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(|m| GroupMember {
                group_id: GroupId(m.group_id),
                user_id: UserId(m.user_id),
                created_at: m.created_at,
            })
            .collect())
    }
}
