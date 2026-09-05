use atlas_api::{
    dtos::{
        AdminUpdateWorkspaceRequest, CreateProjectRequest, CreateWorkspaceRequest, PrincipalDto,
        ProjectDto, UpdateProjectRequest, UpdateWorkspaceRequest, UserDto, WorkspaceDto,
        boards_tasks::{
            ActivityEntryDto, AddAssigneeRequest, AssigneeDto, BoardDto, BoardSummaryDto,
            ChecklistItemDto, ColumnDto, CommentDto, CommentFeedEntryDto, CreateBoardRequest,
            CreateChecklistItemRequest, CreateColumnRequest, CreateCommentRequest,
            CreateReferenceBatchRequest, CreateReferenceBatchResultDto, CreateReferenceRequest,
            CreateSubtaskRequest, CreateTaskRequest, CreateTaskResponseDto, MoveBoardRequest,
            MoveTaskRequest, PromoteChecklistItemRequest, PromotionDto, ReferenceDto,
            RenameTaskAttachmentRequest, SetTaskParentRequest, TaskAttachmentDto, TaskBacklinkDto,
            TaskDto, TaskGraphDto, TaskSummaryDto, UnifiedReferenceDto, UpdateBoardRequest,
            UpdateChecklistItemRequest, UpdateColumnRequest, UpdateCommentRequest,
            UpdateTaskRequest, WorkspaceTaskQueryParams,
        },
        documents::{
            AttachmentDto, BacklinkDto, CommentAttachmentDto, CommentDraftDto, ConflictProblemDto,
            CopyDocumentRequest, CreateDocumentRequest, DocumentCompactDto,
            DocumentContentEditRequest, DocumentContentRangeDto, DocumentContentRangeQuery,
            DocumentContentSearchDto, DocumentContentSearchRequest, DocumentDto,
            DocumentMoveBatchRequest, DocumentMoveBatchResultDto, DocumentSummaryDto,
            FrontmatterDto, MoveDocumentRequest, RenameAttachmentRequest, RevisionContentDto,
            RevisionMetaDto, UpdateContentRequest, UpdateDocumentRequest, WorkspaceAttachmentDto,
        },
        folders::{
            CopyFolderRequest, CreateFolderRequest, FolderDto, MoveFolderRequest,
            RenameFolderRequest,
        },
        lifecycle::{
            PurgeStatusDtoResponse, PurgeTrashItemRequest, RestoreTrashItemRequest, TrashItemDto,
            TrashKindDto,
        },
        property_definitions::{CreatePropertyDefinitionRequest, PropertyDefinitionDto},
        saved_searches::{CreateSavedSearchRequest, RenameSavedSearchRequest, SavedSearchDto},
        search::SearchHitDto,
        semantic_search::SemanticSearchHitDto,
        status_templates::{
            CreateStatusTemplateRequest, PlatformStatusTemplateDto, StatusTemplateDto,
            UpdateStatusTemplateRequest,
        },
        tags::{CreateTagRequest, TagDto, UpdateTagRequest},
        task_views::{CreateTaskViewRequest, TaskViewDto, UpdateTaskViewRequest},
        webhooks::{
            CreateWebhookRequest, UpdateWebhookRequest, WebhookCreatedDto, WebhookDeliveryDto,
            WebhookDto,
        },
    },
    pagination::Page,
    problem::ProblemDetails,
};

use crate::{
    ATTACHMENT_TRANSFER_TIMEOUT, AtlasClient, ClientError, Component, PurgeTrashResult, Req,
    build_comment_feed_path, build_document_list_path, build_document_range_path,
    build_paginated_path, build_search_path, build_semantic_search_path, build_trash_list_path,
    build_webhook_deliveries_path, build_webhooks_list_path, build_workspace_activity_path,
    build_workspace_tasks_path, decode_attachment_content, decode_empty_response,
};

/// The acta-owned methods on [`AtlasClient`]: workspaces, projects, boards
/// and tasks, documents, folders, search, tags, saved searches, status
/// templates, task views, webhooks, and workspace/platform audit feeds —
/// every method whose route is mounted at `/api/v2/acta`. Borrows the root
/// client rather than owning any state of its own, so authentication and
/// CSRF configuration stay single-point on [`AtlasClient`]
/// (INV-SINGLE-AUTH-CONFIG).
pub struct Acta<'a>(pub(crate) &'a AtlasClient);

impl Acta<'_> {
    fn get(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.get(component, relative)
    }

    fn post(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.post(component, relative)
    }

    fn patch(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.patch(component, relative)
    }

    fn put(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.put(component, relative)
    }

    fn delete(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.delete(component, relative)
    }

    async fn decode_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        context: &'static str,
    ) -> Result<T, ClientError> {
        self.0.decode_response(response, context).await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/projects`
    pub async fn create_project(
        &self,
        ws: &str,
        body: CreateProjectRequest,
    ) -> Result<ProjectDto, ClientError> {
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/projects"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_project").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects`
    pub async fn list_projects(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ProjectDto>, ClientError> {
        let path = build_paginated_path(&format!("/workspaces/{ws}/projects"), cursor, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_projects").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}`
    pub async fn get_project(&self, ws: &str, slug: &str) -> Result<ProjectDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{slug}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_project").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/projects/{project_slug}`
    pub async fn update_project(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateProjectRequest,
    ) -> Result<ProjectDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{slug}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_project").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/projects/{project_slug}`
    pub async fn delete_project(&self, ws: &str, slug: &str) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{slug}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces`
    pub async fn create_workspace(&self, name: &str) -> Result<WorkspaceDto, ClientError> {
        let body = CreateWorkspaceRequest {
            name: name.to_string(),
        };
        let response = self
            .post(Component::Acta, "/workspaces")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_workspace").await
    }

    /// `GET /api/v2/acta/workspaces`
    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceDto>, ClientError> {
        let response = self.get(Component::Acta, "/workspaces").send().await?;
        self.decode_response(response, "list_workspaces").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}`
    pub async fn get_workspace(&self, ws: &str) -> Result<WorkspaceDto, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}"))
            .send()
            .await?;
        self.decode_response(response, "get_workspace").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}`
    ///
    /// Renames the workspace display name. The slug is never changed.
    pub async fn update_workspace(
        &self,
        ws: &str,
        body: UpdateWorkspaceRequest,
    ) -> Result<WorkspaceDto, ClientError> {
        let response = self
            .patch(Component::Acta, &format!("/workspaces/{ws}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_workspace").await
    }

    /// `GET /api/v2/acta/admin/workspaces`
    ///
    /// Returns all workspaces in the system. Requires root/admin privileges.
    pub async fn admin_list_workspaces(&self) -> Result<Vec<WorkspaceDto>, ClientError> {
        let response = self
            .get(Component::Acta, "/admin/workspaces")
            .send()
            .await?;
        self.decode_response(response, "admin_list_workspaces")
            .await
    }

    /// `PATCH /api/v2/acta/admin/workspaces/{ws}`
    ///
    /// Updates a workspace's name and/or slug. Requires root/admin privileges.
    pub async fn admin_update_workspace(
        &self,
        ws: &str,
        body: AdminUpdateWorkspaceRequest,
    ) -> Result<WorkspaceDto, ClientError> {
        let response = self
            .patch(Component::Acta, &format!("/admin/workspaces/{ws}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "admin_update_workspace")
            .await
    }

    /// `DELETE /api/v2/acta/admin/workspaces/{ws}`
    ///
    /// Soft-deletes a workspace. Requires root/admin privileges.
    pub async fn admin_delete_workspace(&self, ws: &str) -> Result<(), ClientError> {
        let response = self
            .delete(Component::Acta, &format!("/admin/workspaces/{ws}"))
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/members`
    pub async fn list_workspace_members(&self, ws: &str) -> Result<Vec<PrincipalDto>, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}/members"))
            .send()
            .await?;
        self.decode_response(response, "list_workspace_members")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/members`
    ///
    /// Adds an existing user to the workspace at `role`. Returns the new member
    /// as a `PrincipalDto` on success (HTTP 201).
    pub async fn add_member(
        &self,
        ws: &str,
        user_id: uuid::Uuid,
        role: &str,
    ) -> Result<PrincipalDto, ClientError> {
        use atlas_api::dtos::AddMemberRequest;
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/members"))
            .header("x-atlas-csrf", "1")
            .json(&AddMemberRequest {
                user_id,
                role: role.to_string(),
            })
            .send()
            .await?;
        self.decode_response(response, "add_member").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/assignable-users`
    ///
    /// Lists the active, non-disabled users who are not yet members of the
    /// workspace — the candidates the member picker can add.
    pub async fn list_assignable_users(&self, ws: &str) -> Result<Vec<UserDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/assignable-users"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_assignable_users")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/members/{user_id}`
    pub async fn update_member_role(
        &self,
        ws: &str,
        user_id: uuid::Uuid,
        role: &str,
    ) -> Result<PrincipalDto, ClientError> {
        use atlas_api::dtos::UpdateMemberRoleRequest;
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/members/{user_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&UpdateMemberRoleRequest {
                role: role.to_string(),
            })
            .send()
            .await?;
        self.decode_response(response, "update_member_role").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/members/{user_id}`
    ///
    /// Returns the raw HTTP status code so callers can assert on 204.
    pub async fn remove_member(&self, ws: &str, user_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/members/{user_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: atlas_api::problem::ProblemDetails =
            response.json().await.unwrap_or_else(|_| {
                atlas_api::problem::ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0)
            });
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/search`
    ///
    /// Calls the unified full-text search endpoint. `q` is required; the
    /// remaining parameters are optional and map directly to the query-string
    /// parameters accepted by the server.
    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        ws: &str,
        q: &str,
        type_filter: Option<&str>,
        sort: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
        mode: Option<&str>,
    ) -> Result<Page<SearchHitDto>, ClientError> {
        let path = build_search_path(ws, q, type_filter, sort, cursor, limit, mode);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "search").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/semantic-search`
    pub async fn semantic_search(
        &self,
        ws: &str,
        q: &str,
        type_filter: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<SemanticSearchHitDto>, ClientError> {
        let path = build_semantic_search_path(ws, q, type_filter, cursor, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "semantic_search").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/projects/{project_slug}/folders`
    pub async fn create_folder(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{project_slug}/folders"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_folder").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}/folders`
    pub async fn list_folders(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<FolderDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/projects/{project_slug}/folders"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_folders").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/folders/{folder_id}`
    pub async fn get_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/folders/{folder_id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_folder").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/folders/{folder_id}`
    pub async fn rename_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        body: RenameFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/folders/{folder_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "rename_folder").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/folders/{folder_id}/move`
    pub async fn move_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        body: MoveFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/folders/{folder_id}/move"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_folder").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/folders/{folder_id}/copy`
    pub async fn copy_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        parent_folder_id: Option<uuid::Uuid>,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/folders/{folder_id}/copy"),
            )
            .header("x-atlas-csrf", "1")
            .json(&CopyFolderRequest { parent_folder_id })
            .send()
            .await?;
        self.decode_response(response, "copy_folder").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/folders/{folder_id}`
    pub async fn delete_folder(&self, ws: &str, folder_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/folders/{folder_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents`
    pub async fn create_document(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{project_slug}/documents"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_document").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents`
    pub async fn list_documents(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<DocumentSummaryDto>, ClientError> {
        self.list_documents_with_unfiled_filter(ws, project_slug, cursor, limit, None)
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents?unfiled={bool}`
    ///
    /// `None` lists all documents, `Some(true)` only unfiled documents, and
    /// `Some(false)` only filed documents.
    pub async fn list_documents_with_unfiled_filter(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
        unfiled: Option<bool>,
    ) -> Result<Page<DocumentSummaryDto>, ClientError> {
        self.list_documents_with_options(ws, project_slug, cursor, limit, unfiled, false)
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents?unfiled={bool}&preview={bool}`
    ///
    /// `preview` opts every row into a body preview; listings default to omitting
    /// it so bulk reads stay cheap.
    pub async fn list_documents_with_options(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
        unfiled: Option<bool>,
        preview: bool,
    ) -> Result<Page<DocumentSummaryDto>, ClientError> {
        let path = build_document_list_path(
            &format!("/workspaces/{ws}/projects/{project_slug}/documents"),
            cursor,
            limit,
            unfiled,
            preview,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_documents").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}`
    pub async fn get_document(&self, ws: &str, slug: &str) -> Result<DocumentDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_document").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/compact`
    pub async fn get_document_compact(
        &self,
        ws: &str,
        slug: &str,
    ) -> Result<DocumentCompactDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/compact"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_document_compact").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/content/range`
    pub async fn get_document_content_range(
        &self,
        ws: &str,
        slug: &str,
        query: DocumentContentRangeQuery,
    ) -> Result<DocumentContentRangeDto, ClientError> {
        let path = build_document_range_path(ws, slug, &query);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "get_document_content_range")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/content/search`
    pub async fn search_document_content(
        &self,
        ws: &str,
        slug: &str,
        body: DocumentContentSearchRequest,
    ) -> Result<DocumentContentSearchDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/content/search"),
            )
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "search_document_content")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/documents/{slug}/content/range`
    pub async fn edit_document_content_range(
        &self,
        ws: &str,
        slug: &str,
        body: DocumentContentEditRequest,
    ) -> Result<DocumentCompactDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/content/range"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::CONFLICT {
            let bytes = response.bytes().await?;
            let conflict: ConflictProblemDto =
                serde_json::from_slice(&bytes).map_err(|source| ClientError::Decode {
                    context: "edit_document_content_range_conflict",
                    source,
                })?;
            return Err(ClientError::Conflict(conflict));
        }

        self.decode_response(response, "edit_document_content_range")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/documents/{slug}`
    pub async fn update_document(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_document").await
    }

    /// `PUT /api/v2/acta/workspaces/{ws}/documents/{slug}/content`
    pub async fn update_content(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateContentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .put(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/content"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::CONFLICT {
            let bytes = response.bytes().await?;
            let conflict: ConflictProblemDto =
                serde_json::from_slice(&bytes).map_err(|source| ClientError::Decode {
                    context: "update_content_conflict",
                    source,
                })?;
            return Err(ClientError::Conflict(conflict));
        }

        self.decode_response(response, "update_content").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/documents/{slug}`
    pub async fn delete_document(&self, ws: &str, slug: &str) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/history`
    pub async fn list_document_history(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<RevisionMetaDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/documents/{slug}/history"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_document_history")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/revisions/{seq}`
    pub async fn get_revision_content(
        &self,
        ws: &str,
        slug: &str,
        seq: i64,
    ) -> Result<RevisionContentDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/revisions/{seq}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_revision_content").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/backlinks`
    pub async fn list_backlinks(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<BacklinkDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/documents/{slug}/backlinks"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_backlinks").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/frontmatter`
    pub async fn get_frontmatter(
        &self,
        ws: &str,
        slug: &str,
    ) -> Result<FrontmatterDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/frontmatter"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_frontmatter").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/attachments`
    ///
    /// Uploads raw binary content. Pass `file_name` via the `X-File-Name` header
    /// and the MIME type via `Content-Type`.
    pub async fn upload_attachment(
        &self,
        ws: &str,
        slug: &str,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<AttachmentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/attachments"),
            )
            .header("x-atlas-csrf", "1")
            .header("x-file-name", file_name)
            .header("content-type", content_type)
            .body(data)
            .timeout(ATTACHMENT_TRANSFER_TIMEOUT)
            .send()
            .await?;
        self.decode_response(response, "upload_attachment").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/attachments`
    pub async fn list_attachments(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<AttachmentDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/documents/{slug}/attachments"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_attachments").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/attachments`
    ///
    /// Lists every attachment in the workspace the principal may see, across
    /// notes, tasks, and the comments of either.
    pub async fn list_workspace_attachments(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<WorkspaceAttachmentDto>, ClientError> {
        let path = build_paginated_path(&format!("/workspaces/{ws}/attachments"), cursor, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_workspace_attachments")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/attachments/{attachment_id}`
    ///
    /// Renames the attachment and rewrites the `[[file:…]]` links addressing it.
    pub async fn rename_workspace_attachment(
        &self,
        ws: &str,
        attachment_id: uuid::Uuid,
        body: RenameAttachmentRequest,
    ) -> Result<WorkspaceAttachmentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/attachments/{attachment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "rename_workspace_attachment")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/attachments/{attachment_id}`
    pub async fn download_attachment(
        &self,
        ws: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<Vec<u8>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/attachments/{attachment_id}"),
            )
            .timeout(ATTACHMENT_TRANSFER_TIMEOUT)
            .send()
            .await?;
        if !response.status().is_success() {
            let problem: ProblemDetails = response
                .json()
                .await
                .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
            return Err(ClientError::Api(problem));
        }
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/attachments/{attachment_id}`
    pub async fn delete_attachment(
        &self,
        ws: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/attachments/{attachment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments`
    ///
    /// Uploads a file as `multipart/form-data` with a single part named `file`.
    /// The multipart body is assembled by hand so the client does not need
    /// reqwest's `multipart` feature.
    pub async fn upload_task_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<TaskAttachmentDto, ClientError> {
        let boundary = format!("atlasboundary{}", uuid::Uuid::now_v7().as_simple());

        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/attachments"),
            )
            .header("x-atlas-csrf", "1")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .timeout(ATTACHMENT_TRANSFER_TIMEOUT)
            .send()
            .await?;
        self.decode_response(response, "upload_task_attachment")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments`
    pub async fn list_task_attachments(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<TaskAttachmentDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/attachments"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_task_attachments")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content`
    ///
    /// Returns the streamed bytes together with the response `Content-Type`, so a
    /// caller can assert the content round-trips.
    pub async fn download_task_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<(Vec<u8>, Option<String>), ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!(
                    "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content"
                ),
            )
            .timeout(ATTACHMENT_TRANSFER_TIMEOUT)
            .send()
            .await?;

        if !response.status().is_success() {
            let problem: ProblemDetails = response
                .json()
                .await
                .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
            return Err(ClientError::Api(problem));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let bytes = response.bytes().await?;
        Ok((bytes.to_vec(), content_type))
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}`
    pub async fn rename_task_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        attachment_id: uuid::Uuid,
        body: RenameTaskAttachmentRequest,
    ) -> Result<TaskAttachmentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "rename_task_attachment")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}`
    pub async fn delete_task_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/documents/{slug}/move`
    pub async fn move_document(
        &self,
        ws: &str,
        slug: &str,
        body: MoveDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/move"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_document").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/moves/batch`
    pub async fn move_documents_batch(
        &self,
        ws: &str,
        body: DocumentMoveBatchRequest,
    ) -> Result<Vec<DocumentMoveBatchResultDto>, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/moves/batch"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_documents_batch").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/copy`
    pub async fn copy_document(
        &self,
        ws: &str,
        slug: &str,
        folder_id: Option<uuid::Uuid>,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/copy"),
            )
            .header("x-atlas-csrf", "1")
            .json(&CopyDocumentRequest { folder_id })
            .send()
            .await?;
        self.decode_response(response, "copy_document").await
    }

    /// `GET /api/v2/acta/admin/trash`
    ///
    /// This root/system-admin human-only endpoint lists the five recoverable
    /// resource kinds. API keys are rejected by the server.
    pub async fn list_trash(
        &self,
        workspace_id: Option<uuid::Uuid>,
        kind: Option<TrashKindDto>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<TrashItemDto>, ClientError> {
        let path = build_trash_list_path(workspace_id, kind, cursor, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_trash").await
    }

    /// `POST /api/v2/acta/admin/trash/restore`
    ///
    /// Restores one recoverably deleted resource. This requires a root or
    /// system-admin human session; API keys are rejected by the server.
    pub async fn restore_trash(
        &self,
        kind: TrashKindDto,
        target_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .post(Component::Acta, "/admin/trash/restore")
            .header("x-atlas-csrf", "1")
            .json(&RestoreTrashItemRequest { kind, target_id })
            .send()
            .await?;
        decode_empty_response(response).await
    }

    /// `POST /api/v2/acta/admin/trash/purge`
    ///
    /// Permanently purges a recoverably deleted resource only when `confirm` is
    /// true. A 204 becomes [`PurgeTrashResult::Complete`]; a 202 carries the
    /// durable pending cleanup status.
    pub async fn purge_trash(
        &self,
        kind: TrashKindDto,
        target_id: uuid::Uuid,
        confirm: bool,
    ) -> Result<PurgeTrashResult, ClientError> {
        let response = self
            .post(Component::Acta, "/admin/trash/purge")
            .header("x-atlas-csrf", "1")
            .json(&PurgeTrashItemRequest {
                kind,
                target_id,
                confirm,
            })
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(PurgeTrashResult::Complete);
        }

        let status = self.decode_response(response, "purge_trash").await?;
        Ok(PurgeTrashResult::Pending(status))
    }

    /// `GET /api/v2/acta/admin/trash/purges/{operation_id}`
    pub async fn get_purge_status(
        &self,
        operation_id: uuid::Uuid,
    ) -> Result<PurgeStatusDtoResponse, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/admin/trash/purges/{operation_id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_purge_status").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/webhooks`
    ///
    /// Creates a webhook subscription. The response carries the plaintext HMAC
    /// signing secret (`whsec_…`) exactly once; it is never retrievable again.
    pub async fn create_webhook(
        &self,
        ws: &str,
        body: CreateWebhookRequest,
    ) -> Result<WebhookCreatedDto, ClientError> {
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/webhooks"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_webhook").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/webhooks`
    ///
    /// The list endpoint pages forward with an opaque `after` cursor (not the
    /// generic `cursor` param used elsewhere), so the query string is built here
    /// with the parameter name this route expects.
    pub async fn list_webhooks(
        &self,
        ws: &str,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<WebhookDto>, ClientError> {
        let path = build_webhooks_list_path(ws, after, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_webhooks").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/webhooks/{webhook_id}`
    pub async fn get_webhook(
        &self,
        ws: &str,
        webhook_id: uuid::Uuid,
    ) -> Result<WebhookDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/webhooks/{webhook_id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_webhook").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/webhooks/{webhook_id}`
    ///
    /// PATCH semantics: omitted fields are left unchanged. The signing secret is
    /// never rotated through this endpoint.
    pub async fn update_webhook(
        &self,
        ws: &str,
        webhook_id: uuid::Uuid,
        body: UpdateWebhookRequest,
    ) -> Result<WebhookDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/webhooks/{webhook_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_webhook").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/webhooks/{webhook_id}`
    pub async fn delete_webhook(
        &self,
        ws: &str,
        webhook_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/webhooks/{webhook_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/webhooks/{webhook_id}/deliveries`
    ///
    /// Delivery attempts page newest-first with an opaque `before` cursor, so the
    /// query string is built here with the parameter name this route expects.
    pub async fn list_webhook_deliveries(
        &self,
        ws: &str,
        webhook_id: uuid::Uuid,
        before: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<WebhookDeliveryDto>, ClientError> {
        let path = build_webhook_deliveries_path(ws, webhook_id, before, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_webhook_deliveries")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/projects/{project_slug}/boards`
    pub async fn create_board(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateBoardRequest,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/projects/{project_slug}/boards"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_board").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/projects/{project_slug}/boards`
    pub async fn list_boards(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<BoardSummaryDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/projects/{project_slug}/boards"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_boards").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/boards/{board_id}`
    pub async fn get_board(&self, ws: &str, board_id: uuid::Uuid) -> Result<BoardDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_board").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/boards/{board_id}`
    pub async fn update_board(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: UpdateBoardRequest,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_board").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/boards/{board_id}/move`
    pub async fn move_board(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: MoveBoardRequest,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/move"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_board").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/boards/{board_id}`
    pub async fn delete_board(&self, ws: &str, board_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/columns`
    pub async fn create_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: CreateColumnRequest,
    ) -> Result<ColumnDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/columns"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_column").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/boards/{board_id}/columns`
    pub async fn list_columns(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
    ) -> Result<Vec<ColumnDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/columns"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_columns").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tags`
    ///
    /// Idempotent by case-insensitive name: an existing tag is returned with 200,
    /// a new one with 201. Both are surfaced as a successful `TagDto`.
    pub async fn create_tag(
        &self,
        ws: &str,
        body: CreateTagRequest,
    ) -> Result<TagDto, ClientError> {
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/tags"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_tag").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tags`
    pub async fn list_tags(&self, ws: &str) -> Result<Vec<TagDto>, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}/tags"))
            .send()
            .await?;
        self.decode_response(response, "list_tags").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tags/used`
    pub async fn list_used_labels(&self, ws: &str) -> Result<Vec<String>, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}/tags/used"))
            .send()
            .await?;
        self.decode_response(response, "list_used_labels").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/tags/{tag_id}`
    ///
    /// Updates a tag's name and/or color. Returns the updated tag.
    pub async fn update_tag(
        &self,
        ws: &str,
        tag_id: uuid::Uuid,
        body: UpdateTagRequest,
    ) -> Result<TagDto, ClientError> {
        let response = self
            .patch(Component::Acta, &format!("/workspaces/{ws}/tags/{tag_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_tag").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tags/{tag_id}`
    ///
    /// Soft-deletes a tag. Task label strings are preserved.
    pub async fn delete_tag(&self, ws: &str, tag_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(Component::Acta, &format!("/workspaces/{ws}/tags/{tag_id}"))
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/property-definitions`
    ///
    /// Optionally filters by applicability (`task` | `document` | `both`).
    pub async fn list_property_definitions(
        &self,
        ws: &str,
        applies_to: Option<&str>,
    ) -> Result<Vec<PropertyDefinitionDto>, ClientError> {
        let mut path = format!("/workspaces/{ws}/property-definitions");
        if let Some(applies_to) = applies_to {
            path.push_str(&format!("?applies_to={applies_to}"));
        }
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_property_definitions")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/property-definitions`
    pub async fn create_property_definition(
        &self,
        ws: &str,
        body: CreatePropertyDefinitionRequest,
    ) -> Result<PropertyDefinitionDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/property-definitions"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_property_definition")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/property-definitions/{property_definition_id}`
    pub async fn delete_property_definition(
        &self,
        ws: &str,
        property_definition_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/property-definitions/{property_definition_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/saved-searches`
    pub async fn create_saved_search(
        &self,
        ws: &str,
        body: CreateSavedSearchRequest,
    ) -> Result<SavedSearchDto, ClientError> {
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/saved-searches"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_saved_search").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/saved-searches`
    pub async fn list_saved_searches(&self, ws: &str) -> Result<Vec<SavedSearchDto>, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}/saved-searches"))
            .send()
            .await?;
        self.decode_response(response, "list_saved_searches").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/saved-searches/{id}`
    pub async fn rename_saved_search(
        &self,
        ws: &str,
        id: uuid::Uuid,
        body: RenameSavedSearchRequest,
    ) -> Result<SavedSearchDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/saved-searches/{id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "rename_saved_search").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/saved-searches/{id}`
    pub async fn delete_saved_search(&self, ws: &str, id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/saved-searches/{id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/task-views`
    pub async fn list_task_views(&self, ws: &str) -> Result<Vec<TaskViewDto>, ClientError> {
        let response = self
            .get(Component::Acta, &format!("/workspaces/{ws}/task-views"))
            .send()
            .await?;
        self.decode_response(response, "list_task_views").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/task-views`
    pub async fn create_task_view(
        &self,
        ws: &str,
        body: CreateTaskViewRequest,
    ) -> Result<TaskViewDto, ClientError> {
        let response = self
            .post(Component::Acta, &format!("/workspaces/{ws}/task-views"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_task_view").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/task-views/{id}`
    pub async fn get_task_view(
        &self,
        ws: &str,
        id: uuid::Uuid,
    ) -> Result<TaskViewDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/task-views/{id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_task_view").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/task-views/{id}`
    pub async fn update_task_view(
        &self,
        ws: &str,
        id: uuid::Uuid,
        body: UpdateTaskViewRequest,
    ) -> Result<TaskViewDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/task-views/{id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_task_view").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/task-views/{id}`
    pub async fn delete_task_view(&self, ws: &str, id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/task-views/{id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/status-templates`
    pub async fn list_status_templates(
        &self,
        ws: &str,
    ) -> Result<Vec<StatusTemplateDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/status-templates"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_status_templates")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/status-templates`
    pub async fn create_status_template(
        &self,
        ws: &str,
        body: CreateStatusTemplateRequest,
    ) -> Result<StatusTemplateDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/status-templates"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_status_template")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/status-templates/{template_id}`
    pub async fn update_status_template(
        &self,
        ws: &str,
        template_id: uuid::Uuid,
        body: UpdateStatusTemplateRequest,
    ) -> Result<StatusTemplateDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/status-templates/{template_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_status_template")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/status-templates/{template_id}`
    pub async fn delete_status_template(
        &self,
        ws: &str,
        template_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/status-templates/{template_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/admin/status-templates`
    pub async fn list_platform_status_templates(
        &self,
    ) -> Result<Vec<PlatformStatusTemplateDto>, ClientError> {
        let response = self
            .get(Component::Acta, "/admin/status-templates")
            .send()
            .await?;
        self.decode_response(response, "list_platform_status_templates")
            .await
    }

    /// `POST /api/v2/acta/admin/status-templates`
    pub async fn create_platform_status_template(
        &self,
        body: CreateStatusTemplateRequest,
    ) -> Result<PlatformStatusTemplateDto, ClientError> {
        let response = self
            .post(Component::Acta, "/admin/status-templates")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_platform_status_template")
            .await
    }

    /// `PATCH /api/v2/acta/admin/status-templates/{template_id}`
    pub async fn update_platform_status_template(
        &self,
        template_id: uuid::Uuid,
        body: UpdateStatusTemplateRequest,
    ) -> Result<PlatformStatusTemplateDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/admin/status-templates/{template_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_platform_status_template")
            .await
    }

    /// `DELETE /api/v2/acta/admin/status-templates/{template_id}`
    pub async fn delete_platform_status_template(
        &self,
        template_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/admin/status-templates/{template_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/apply-status-templates`
    pub async fn apply_status_templates(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
    ) -> Result<Vec<atlas_api::dtos::boards_tasks::ColumnDto>, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/apply-status-templates"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "apply_status_templates")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/boards/{board_id}/columns/{column_id}`
    pub async fn update_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        column_id: uuid::Uuid,
        body: UpdateColumnRequest,
    ) -> Result<ColumnDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/columns/{column_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_column").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/boards/{board_id}/columns/{column_id}`
    pub async fn delete_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        column_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/columns/{column_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/tasks`
    pub async fn create_task(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: CreateTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        Ok(self
            .create_task_with_references(ws, board_id, body)
            .await?
            .task)
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/tasks`
    pub async fn create_task_with_references(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: CreateTaskRequest,
    ) -> Result<CreateTaskResponseDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/tasks"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_task").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/boards/{board_id}/tasks`
    pub async fn list_tasks(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<TaskSummaryDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/boards/{board_id}/tasks"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_tasks").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks`
    pub async fn list_workspace_tasks(
        &self,
        ws: &str,
        query: &WorkspaceTaskQueryParams,
    ) -> Result<Page<TaskSummaryDto>, ClientError> {
        let path = build_workspace_tasks_path(ws, query);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_workspace_tasks").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}`
    pub async fn get_task(&self, ws: &str, readable_id: &str) -> Result<TaskDto, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}"),
            )
            .send()
            .await?;
        self.decode_response(response, "get_task").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/tasks/{readable_id}`
    pub async fn update_task(
        &self,
        ws: &str,
        readable_id: &str,
        body: UpdateTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_task").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}`
    pub async fn delete_task(&self, ws: &str, readable_id: &str) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/move`
    pub async fn move_task(
        &self,
        ws: &str,
        readable_id: &str,
        body: MoveTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/move"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_task").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/assignees`
    pub async fn list_assignees(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<AssigneeDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/assignees"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_assignees").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/assignees`
    pub async fn add_assignee(
        &self,
        ws: &str,
        readable_id: &str,
        body: AddAssigneeRequest,
    ) -> Result<AssigneeDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/assignees"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "add_assignee").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}`
    pub async fn remove_assignee(
        &self,
        ws: &str,
        readable_id: &str,
        assignee_ref: &str,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/references`
    pub async fn list_references(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<UnifiedReferenceDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/references"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_references").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/references`
    pub async fn create_reference(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateReferenceRequest,
    ) -> Result<ReferenceDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/references"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_reference").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/references/batch`
    pub async fn create_reference_batch(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateReferenceBatchRequest,
    ) -> Result<Vec<CreateReferenceBatchResultDto>, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/references/batch"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_reference_batch")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}`
    pub async fn delete_reference(
        &self,
        ws: &str,
        readable_id: &str,
        reference_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/backlinks`
    pub async fn list_task_backlinks(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Page<TaskBacklinkDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/backlinks"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_task_backlinks").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/archive`
    pub async fn archive_board(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/archive"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "archive_board").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/boards/{board_id}/unarchive`
    pub async fn unarchive_board(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/boards/{board_id}/unarchive"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "unarchive_board").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/graph`
    pub async fn get_task_graph(
        &self,
        ws: &str,
        readable_id: &str,
        depth: Option<u32>,
    ) -> Result<TaskGraphDto, ClientError> {
        let path = match depth {
            Some(depth) => format!("/workspaces/{ws}/tasks/{readable_id}/graph?depth={depth}"),
            None => format!("/workspaces/{ws}/tasks/{readable_id}/graph"),
        };
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "get_task_graph").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/checklist`
    pub async fn list_checklist(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<ChecklistItemDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/checklist"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_checklist").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/checklist`
    pub async fn create_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateChecklistItemRequest,
    ) -> Result<ChecklistItemDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/checklist"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_checklist_item")
            .await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}`
    pub async fn update_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
        body: UpdateChecklistItemRequest,
    ) -> Result<ChecklistItemDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_checklist_item")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}`
    pub async fn delete_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote`
    pub async fn promote_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
        body: PromoteChecklistItemRequest,
    ) -> Result<PromotionDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "promote_checklist_item")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/subtasks`
    pub async fn list_subtasks(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<TaskSummaryDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/subtasks"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_subtasks").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/subtasks`
    pub async fn create_subtask(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateSubtaskRequest,
    ) -> Result<CreateTaskResponseDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/subtasks"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_subtask").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/promote`
    pub async fn promote_subtask(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/promote"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "promote_subtask").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/parent`
    pub async fn set_task_parent(
        &self,
        ws: &str,
        readable_id: &str,
        body: SetTaskParentRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/parent"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "set_task_parent").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/activity`
    pub async fn list_activity(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Page<ActivityEntryDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/activity"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_activity").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments`
    pub async fn list_comments(
        &self,
        ws: &str,
        readable_id: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<CommentDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/tasks/{readable_id}/comments"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_comments").await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments?feed=full`
    pub async fn list_comment_feed(
        &self,
        ws: &str,
        readable_id: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<CommentFeedEntryDto>, ClientError> {
        let path = build_comment_feed_path(
            &format!("/workspaces/{ws}/tasks/{readable_id}/comments"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_comment_feed").await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments`
    pub async fn upload_task_comment_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<CommentAttachmentDto, ClientError> {
        let boundary = format!("atlasboundary{}", uuid::Uuid::now_v7().as_simple());
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments"),
            )
            .header("x-atlas-csrf", "1")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?;
        self.decode_response(response, "upload_task_comment_attachment")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comment-drafts`
    pub async fn create_task_comment_draft(
        &self,
        ws: &str,
        readable_id: &str,
        create_token: uuid::Uuid,
    ) -> Result<CommentDraftDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comment-drafts"),
            )
            .header("x-atlas-csrf", "1")
            .header("x-create-token", create_token.to_string())
            .send()
            .await?;
        self.decode_response(response, "create_task_comment_draft")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments`
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_task_draft_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        draft_id: uuid::Uuid,
        upload_token: uuid::Uuid,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<CommentAttachmentDto, ClientError> {
        let boundary = format!("atlasboundary{}", uuid::Uuid::now_v7().as_simple());
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let response = self
            .post(
                Component::Acta,
                &format!(
                    "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments"
                ),
            )
            .header("x-atlas-csrf", "1")
            .header("x-upload-token", upload_token.to_string())
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?;
        self.decode_response(response, "upload_task_draft_attachment")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}`
    pub async fn cancel_task_comment_draft(
        &self,
        ws: &str,
        readable_id: &str,
        draft_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        decode_empty_response(response).await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments`
    pub async fn list_task_comment_attachments(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
    ) -> Result<Vec<CommentAttachmentDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_task_comment_attachments")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content`
    pub async fn download_task_comment_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
        attachment_id: uuid::Uuid,
    ) -> Result<(Vec<u8>, Option<String>), ClientError> {
        let response = self
            .get(Component::Acta, &format!(
                "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content"
            ))
            .send()
            .await?;
        decode_attachment_content(response).await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}`
    pub async fn delete_task_comment_attachment(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
        attachment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(Component::Acta, &format!(
                "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}"
            ))
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        decode_empty_response(response).await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments`
    pub async fn add_comment(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateCommentRequest,
    ) -> Result<CommentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comments"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "add_comment").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}`
    pub async fn update_comment(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
        body: UpdateCommentRequest,
    ) -> Result<CommentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_comment").await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}`
    pub async fn delete_comment(
        &self,
        ws: &str,
        readable_id: &str,
        comment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/comments`
    pub async fn list_document_comments(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<CommentDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/documents/{slug}/comments"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_document_comments")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/comments?feed=full`
    pub async fn list_document_comment_feed(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<CommentFeedEntryDto>, ClientError> {
        let path = build_comment_feed_path(
            &format!("/workspaces/{ws}/documents/{slug}/comments"),
            cursor,
            limit,
        );
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_document_comment_feed")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments`
    pub async fn upload_document_comment_attachment(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<CommentAttachmentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments"),
            )
            .header("x-atlas-csrf", "1")
            .header("x-file-name", file_name)
            .header("content-type", content_type)
            .body(data)
            .send()
            .await?;
        self.decode_response(response, "upload_document_comment_attachment")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/comment-drafts`
    pub async fn create_document_comment_draft(
        &self,
        ws: &str,
        slug: &str,
        create_token: uuid::Uuid,
    ) -> Result<CommentDraftDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comment-drafts"),
            )
            .header("x-atlas-csrf", "1")
            .header("x-create-token", create_token.to_string())
            .send()
            .await?;
        self.decode_response(response, "create_document_comment_draft")
            .await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments`
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_document_draft_attachment(
        &self,
        ws: &str,
        slug: &str,
        draft_id: uuid::Uuid,
        upload_token: uuid::Uuid,
        file_name: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<CommentAttachmentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments"),
            )
            .header("x-atlas-csrf", "1")
            .header("x-upload-token", upload_token.to_string())
            .header("x-file-name", file_name)
            .header("content-type", content_type)
            .body(data)
            .send()
            .await?;
        self.decode_response(response, "upload_document_draft_attachment")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}`
    pub async fn cancel_document_comment_draft(
        &self,
        ws: &str,
        slug: &str,
        draft_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        decode_empty_response(response).await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments`
    pub async fn list_document_comment_attachments(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
    ) -> Result<Vec<CommentAttachmentDto>, ClientError> {
        let response = self
            .get(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_document_comment_attachments")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}`
    pub async fn download_document_comment_attachment(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
        attachment_id: uuid::Uuid,
    ) -> Result<(Vec<u8>, Option<String>), ClientError> {
        let response = self
            .get(Component::Acta, &format!(
                "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}"
            ))
            .send()
            .await?;
        decode_attachment_content(response).await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}`
    pub async fn delete_document_comment_attachment(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
        attachment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(Component::Acta, &format!(
                "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}"
            ))
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        decode_empty_response(response).await
    }

    /// `POST /api/v2/acta/workspaces/{ws}/documents/{slug}/comments`
    pub async fn add_document_comment(
        &self,
        ws: &str,
        slug: &str,
        body: CreateCommentRequest,
    ) -> Result<CommentDto, ClientError> {
        let response = self
            .post(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comments"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "add_document_comment").await
    }

    /// `PATCH /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}`
    pub async fn update_document_comment(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
        body: UpdateCommentRequest,
    ) -> Result<CommentDto, ClientError> {
        let response = self
            .patch(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comments/{comment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_document_comment")
            .await
    }

    /// `DELETE /api/v2/acta/workspaces/{ws}/documents/{slug}/comments/{comment_id}`
    pub async fn delete_document_comment(
        &self,
        ws: &str,
        slug: &str,
        comment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Acta,
                &format!("/workspaces/{ws}/documents/{slug}/comments/{comment_id}"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        let problem: ProblemDetails = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        Err(ClientError::Api(problem))
    }

    /// `GET /api/v2/acta/workspaces/{ws}/activity`
    pub async fn list_workspace_activity(
        &self,
        ws: &str,
        actor: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ActivityEntryDto>, ClientError> {
        let path = build_workspace_activity_path(ws, actor, from, to, None, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_workspace_activity")
            .await
    }

    /// `GET /api/v2/acta/workspaces/{ws}/activity`
    pub async fn list_workspace_activity_with_cursor(
        &self,
        ws: &str,
        actor: Option<&str>,
        from: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ActivityEntryDto>, ClientError> {
        let path = build_workspace_activity_path(ws, actor, from, None, cursor, limit);
        let response = self.get(Component::Acta, &path).send().await?;
        self.decode_response(response, "list_workspace_activity_with_cursor")
            .await
    }
}
