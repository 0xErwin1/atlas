use crate::persistence::entities::boards_tasks::actor_from_columns;
use atlas_domain::entities::comments::Comment;
use atlas_domain::ids::{CommentId, DocumentId, TaskId, WorkspaceId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod comment {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub task_id: Option<Uuid>,
        pub document_id: Option<Uuid>,
        pub body: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn comment_from(m: comment::Model) -> Comment {
    Comment {
        id: CommentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        task_id: m.task_id.map(TaskId),
        document_id: m.document_id.map(DocumentId),
        body: m.body,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn base_model() -> comment::Model {
        comment::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            task_id: Some(Uuid::now_v7()),
            document_id: None,
            body: "hello".into(),
            created_by_user_id: Some(Uuid::now_v7()),
            created_by_api_key_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    #[test]
    fn comment_from_roundtrips_task_owner_and_body() {
        let m = base_model();
        let task_id = m.task_id;
        let body = m.body.clone();

        let comment = comment_from(m);

        assert_eq!(comment.task_id.map(|id| id.0), task_id);
        assert!(comment.document_id.is_none());
        assert_eq!(comment.body, body);
    }

    #[test]
    fn comment_from_resolves_api_key_author() {
        let mut m = base_model();
        let key_id = Uuid::now_v7();
        m.created_by_user_id = None;
        m.created_by_api_key_id = Some(key_id);

        let comment = comment_from(m);

        assert_eq!(
            comment.created_by,
            atlas_domain::Actor::ApiKey(atlas_domain::ApiKeyId(key_id))
        );
    }
}
