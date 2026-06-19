#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, ChangePasswordRequest, CreateApiKeyRequest, CreateGrantRequest,
        CreateProjectRequest, CreateUserRequest, CreateWorkspaceRequest, GrantDto, HealthResponse,
        LoginRequest, LoginResponse, MeResponse, PrincipalDto, ProjectDto, ResetPasswordRequest,
        ServerMetaDto, UiStateDto, UpdateMeRequest, UpdateProjectRequest, UpdateUiStateRequest,
        UserDto, WorkspaceDto,
        boards_tasks::{
            ActivityEntryDto, AddAssigneeRequest, AssigneeDto, BoardDto, BoardSummaryDto,
            ChecklistItemDto, ColumnDto, CreateBoardRequest, CreateChecklistItemRequest,
            CreateColumnRequest, CreateReferenceRequest, CreateSubtaskRequest, CreateTaskRequest,
            MoveTaskRequest,
            PromoteChecklistItemRequest, PromotionDto, ReferenceDto, TaskBacklinkDto, TaskDto,
            TaskSummaryDto, UpdateBoardRequest, UpdateChecklistItemRequest, UpdateColumnRequest,
            UpdateTaskRequest,
        },
        documents::{
            AttachmentDto, BacklinkDto, ConflictProblemDto, CopyDocumentRequest,
            CreateDocumentRequest, DocumentDto, DocumentSummaryDto, FrontmatterDto,
            MoveDocumentRequest, RevisionContentDto, RevisionMetaDto, UpdateContentRequest,
            UpdateDocumentRequest,
        },
        folders::{
            CopyFolderRequest, CreateFolderRequest, FolderDto, MoveFolderRequest,
            RenameFolderRequest,
        },
        search::SearchHitDto,
    },
    pagination::Page,
    problem::ProblemDetails,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("API error {}: {}", .0.status, .0.title)]
    Api(ProblemDetails),
    /// CAS revision conflict (HTTP 409) carrying the head revision and the patch
    /// from the client's stale base to the current content, so callers can apply
    /// the patch and retry.
    #[error("revision conflict: current_seq={}", .0.current_seq)]
    Conflict(ConflictProblemDto),
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("decode error in {context}: {source}")]
    Decode {
        context: &'static str,
        source: serde_json::Error,
    },
}

pub struct AtlasClient {
    base_url: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl AtlasClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
            token: None,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = Some(token.into());
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.get(format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.post(format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.patch(format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.put(format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.http.delete(format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    async fn decode_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        context: &'static str,
    ) -> Result<T, ClientError> {
        if !response.status().is_success() {
            let problem: ProblemDetails = response
                .json()
                .await
                .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
            return Err(ClientError::Api(problem));
        }

        let body = response.bytes().await?;
        serde_json::from_slice(&body).map_err(|source| ClientError::Decode { context, source })
    }

    /// `GET /health`
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let response = self.get("/health").send().await?;
        self.decode_response(response, "health").await
    }

    /// `POST /v1/auth/login`
    ///
    /// On success, stores the returned session token in `self.token`.
    pub async fn login(&mut self, body: LoginRequest) -> Result<LoginResponse, ClientError> {
        let response = self.post("/v1/auth/login").json(&body).send().await?;
        let login: LoginResponse = self.decode_response(response, "login").await?;
        self.token = Some(login.token.clone());
        Ok(login)
    }

    /// `GET /v1/auth/me`
    pub async fn me(&self) -> Result<MeResponse, ClientError> {
        let response = self.get("/v1/auth/me").send().await?;
        self.decode_response(response, "me").await
    }

    /// `POST /v1/auth/change-password`
    pub async fn change_password(&self, body: ChangePasswordRequest) -> Result<(), ClientError> {
        let response = self
            .post("/v1/auth/change-password")
            .header("x-atlas-csrf", "1")
            .json(&body)
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

    /// `PATCH /v1/users/me`
    pub async fn update_me(&self, body: UpdateMeRequest) -> Result<UserDto, ClientError> {
        let response = self
            .patch("/v1/users/me")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_me").await
    }

    /// `GET /v1/me/ui-state`
    ///
    /// Returns the current user's stored UI state object (an empty object when
    /// no state has been saved yet).
    pub async fn get_ui_state(&self) -> Result<serde_json::Value, ClientError> {
        let response = self.get("/v1/me/ui-state").send().await?;
        let dto: UiStateDto = self.decode_response(response, "get_ui_state").await?;
        Ok(dto.state)
    }

    /// `PUT /v1/me/ui-state`
    ///
    /// Upserts the current user's UI state and returns the stored object.
    pub async fn set_ui_state(
        &self,
        state: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let body = UpdateUiStateRequest {
            state: state.clone(),
        };
        let response = self
            .put("/v1/me/ui-state")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        let dto: UiStateDto = self.decode_response(response, "set_ui_state").await?;
        Ok(dto.state)
    }

    /// `GET /v1/meta`
    pub async fn server_meta(&self) -> Result<ServerMetaDto, ClientError> {
        let response = self.get("/v1/meta").send().await?;
        self.decode_response(response, "server_meta").await
    }

    /// `GET /v1/users`
    pub async fn list_users(&self) -> Result<Vec<UserDto>, ClientError> {
        let response = self.get("/v1/users").send().await?;
        self.decode_response(response, "list_users").await
    }

    /// `POST /v1/users`
    pub async fn create_user(&self, body: CreateUserRequest) -> Result<UserDto, ClientError> {
        let response = self.post("/v1/users").json(&body).send().await?;
        self.decode_response(response, "create_user").await
    }

    /// `POST /v1/users/{user_id}/disable`
    pub async fn disable_user(&self, user_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .post(&format!("/v1/users/{user_id}/disable"))
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

    /// `POST /v1/users/{user_id}/enable`
    pub async fn enable_user(&self, user_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .post(&format!("/v1/users/{user_id}/enable"))
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

    /// `POST /v1/users/{user_id}/reset-password`
    pub async fn reset_user_password(
        &self,
        user_id: uuid::Uuid,
        new_password: impl Into<String>,
    ) -> Result<(), ClientError> {
        let response = self
            .post(&format!("/v1/users/{user_id}/reset-password"))
            .header("x-atlas-csrf", "1")
            .json(&ResetPasswordRequest {
                new_password: new_password.into(),
            })
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

    /// `POST /v1/workspaces/{ws}/api-keys`
    pub async fn create_api_key(
        &self,
        ws: &str,
        body: CreateApiKeyRequest,
    ) -> Result<ApiKeyCreated, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/api-keys"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_api_key").await
    }

    /// `GET /v1/workspaces/{ws}/api-keys`
    pub async fn list_api_keys(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ApiKeyDto>, ClientError> {
        let path = build_paginated_path(&format!("/v1/workspaces/{ws}/api-keys"), cursor, limit);
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_api_keys").await
    }

    /// `POST /v1/workspaces/{ws}/api-keys/{key_id}/revoke`
    pub async fn revoke_api_key(&self, ws: &str, key_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/api-keys/{key_id}/revoke"))
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

    /// `POST /v1/workspaces/{ws}/projects`
    pub async fn create_project(
        &self,
        ws: &str,
        body: CreateProjectRequest,
    ) -> Result<ProjectDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/projects"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_project").await
    }

    /// `GET /v1/workspaces/{ws}/projects`
    pub async fn list_projects(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ProjectDto>, ClientError> {
        let path = build_paginated_path(&format!("/v1/workspaces/{ws}/projects"), cursor, limit);
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_projects").await
    }

    /// `GET /v1/workspaces/{ws}/projects/{project_slug}`
    pub async fn get_project(&self, ws: &str, slug: &str) -> Result<ProjectDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/projects/{slug}"))
            .send()
            .await?;
        self.decode_response(response, "get_project").await
    }

    /// `PATCH /v1/workspaces/{ws}/projects/{project_slug}`
    pub async fn update_project(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateProjectRequest,
    ) -> Result<ProjectDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/projects/{slug}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_project").await
    }

    /// `DELETE /v1/workspaces/{ws}/projects/{project_slug}`
    pub async fn delete_project(&self, ws: &str, slug: &str) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/projects/{slug}"))
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

    /// `POST /v1/workspaces/{ws}/projects/{slug}/grants`
    pub async fn create_project_grant(
        &self,
        ws: &str,
        slug: &str,
        body: CreateGrantRequest,
    ) -> Result<GrantDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/projects/{slug}/grants"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_project_grant").await
    }

    /// `GET /v1/workspaces/{ws}/projects/{slug}/grants`
    pub async fn list_project_grants(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<GrantDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/projects/{slug}/grants"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_project_grants").await
    }

    /// `DELETE /v1/workspaces/{ws}/projects/{slug}/grants/{grant_id}`
    pub async fn delete_project_grant(
        &self,
        ws: &str,
        slug: &str,
        grant_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!(
                "/v1/workspaces/{ws}/projects/{slug}/grants/{grant_id}"
            ))
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

    /// `POST /v1/workspaces/{ws}/grants`
    pub async fn create_workspace_grant(
        &self,
        ws: &str,
        body: CreateGrantRequest,
    ) -> Result<GrantDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/grants"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_workspace_grant")
            .await
    }

    /// `GET /v1/workspaces/{ws}/grants`
    pub async fn list_workspace_grants(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<GrantDto>, ClientError> {
        let path = build_paginated_path(&format!("/v1/workspaces/{ws}/grants"), cursor, limit);
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_workspace_grants")
            .await
    }

    /// `DELETE /v1/workspaces/{ws}/grants/{grant_id}`
    pub async fn delete_workspace_grant(
        &self,
        ws: &str,
        grant_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/grants/{grant_id}"))
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

    /// `POST /v1/workspaces`
    pub async fn create_workspace(&self, name: &str) -> Result<WorkspaceDto, ClientError> {
        let body = CreateWorkspaceRequest {
            name: name.to_string(),
        };
        let response = self
            .post("/v1/workspaces")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_workspace").await
    }

    /// `GET /v1/workspaces`
    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceDto>, ClientError> {
        let response = self.get("/v1/workspaces").send().await?;
        self.decode_response(response, "list_workspaces").await
    }

    /// `GET /v1/workspaces/{ws}`
    pub async fn get_workspace(&self, ws: &str) -> Result<WorkspaceDto, ClientError> {
        let response = self.get(&format!("/v1/workspaces/{ws}")).send().await?;
        self.decode_response(response, "get_workspace").await
    }

    /// `GET /v1/workspaces/{ws}/members`
    pub async fn list_workspace_members(&self, ws: &str) -> Result<Vec<PrincipalDto>, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/members"))
            .send()
            .await?;
        self.decode_response(response, "list_workspace_members")
            .await
    }

    /// `GET /v1/workspaces/{ws}/search`
    ///
    /// Calls the unified full-text search endpoint. `q` is required; the
    /// remaining parameters are optional and map directly to the query-string
    /// parameters accepted by the server.
    pub async fn search(
        &self,
        ws: &str,
        q: &str,
        type_filter: Option<&str>,
        sort: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<SearchHitDto>, ClientError> {
        let path = build_search_path(ws, q, type_filter, sort, cursor, limit);
        let response = self.get(&path).send().await?;
        self.decode_response(response, "search").await
    }

    /// `POST /v1/workspaces/{ws}/projects/{project_slug}/folders`
    pub async fn create_folder(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/projects/{project_slug}/folders"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_folder").await
    }

    /// `GET /v1/workspaces/{ws}/projects/{project_slug}/folders`
    pub async fn list_folders(
        &self,
        ws: &str,
        project_slug: &str,
    ) -> Result<Page<FolderDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/projects/{project_slug}/folders"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_folders").await
    }

    /// `GET /v1/workspaces/{ws}/folders/{folder_id}`
    pub async fn get_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/folders/{folder_id}"))
            .send()
            .await?;
        self.decode_response(response, "get_folder").await
    }

    /// `PATCH /v1/workspaces/{ws}/folders/{folder_id}`
    pub async fn rename_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        body: RenameFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/folders/{folder_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "rename_folder").await
    }

    /// `PATCH /v1/workspaces/{ws}/folders/{folder_id}/move`
    pub async fn move_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        body: MoveFolderRequest,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/folders/{folder_id}/move"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_folder").await
    }

    /// `POST /v1/workspaces/{ws}/folders/{folder_id}/copy`
    pub async fn copy_folder(
        &self,
        ws: &str,
        folder_id: uuid::Uuid,
        parent_folder_id: Option<uuid::Uuid>,
    ) -> Result<FolderDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/folders/{folder_id}/copy"))
            .header("x-atlas-csrf", "1")
            .json(&CopyFolderRequest { parent_folder_id })
            .send()
            .await?;
        self.decode_response(response, "copy_folder").await
    }

    /// `DELETE /v1/workspaces/{ws}/folders/{folder_id}`
    pub async fn delete_folder(&self, ws: &str, folder_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/folders/{folder_id}"))
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

    /// `POST /v1/workspaces/{ws}/projects/{project_slug}/documents`
    pub async fn create_document(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/projects/{project_slug}/documents"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_document").await
    }

    /// `GET /v1/workspaces/{ws}/projects/{project_slug}/documents`
    pub async fn list_documents(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<DocumentSummaryDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/projects/{project_slug}/documents"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_documents").await
    }

    /// `GET /v1/workspaces/{ws}/documents/{slug}`
    pub async fn get_document(&self, ws: &str, slug: &str) -> Result<DocumentDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/documents/{slug}"))
            .send()
            .await?;
        self.decode_response(response, "get_document").await
    }

    /// `PATCH /v1/workspaces/{ws}/documents/{slug}`
    pub async fn update_document(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/documents/{slug}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_document").await
    }

    /// `PUT /v1/workspaces/{ws}/documents/{slug}/content`
    pub async fn update_content(
        &self,
        ws: &str,
        slug: &str,
        body: UpdateContentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .put(&format!("/v1/workspaces/{ws}/documents/{slug}/content"))
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

    /// `DELETE /v1/workspaces/{ws}/documents/{slug}`
    pub async fn delete_document(&self, ws: &str, slug: &str) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/documents/{slug}"))
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

    /// `GET /v1/workspaces/{ws}/documents/{slug}/history`
    pub async fn list_document_history(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<RevisionMetaDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/documents/{slug}/history"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_document_history")
            .await
    }

    /// `GET /v1/workspaces/{ws}/documents/{slug}/revisions/{seq}`
    pub async fn get_revision_content(
        &self,
        ws: &str,
        slug: &str,
        seq: i64,
    ) -> Result<RevisionContentDto, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/documents/{slug}/revisions/{seq}"
            ))
            .send()
            .await?;
        self.decode_response(response, "get_revision_content").await
    }

    /// `GET /v1/workspaces/{ws}/documents/{slug}/backlinks`
    pub async fn list_backlinks(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<BacklinkDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/documents/{slug}/backlinks"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_backlinks").await
    }

    /// `GET /v1/workspaces/{ws}/documents/{slug}/frontmatter`
    pub async fn get_frontmatter(
        &self,
        ws: &str,
        slug: &str,
    ) -> Result<FrontmatterDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/documents/{slug}/frontmatter"))
            .send()
            .await?;
        self.decode_response(response, "get_frontmatter").await
    }

    /// `POST /v1/workspaces/{ws}/documents/{slug}/attachments`
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
            .post(&format!("/v1/workspaces/{ws}/documents/{slug}/attachments"))
            .header("x-atlas-csrf", "1")
            .header("x-file-name", file_name)
            .header("content-type", content_type)
            .body(data)
            .send()
            .await?;
        self.decode_response(response, "upload_attachment").await
    }

    /// `GET /v1/workspaces/{ws}/documents/{slug}/attachments`
    pub async fn list_attachments(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<AttachmentDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/documents/{slug}/attachments"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_attachments").await
    }

    /// `GET /v1/workspaces/{ws}/attachments/{attachment_id}`
    pub async fn download_attachment(
        &self,
        ws: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<Vec<u8>, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/attachments/{attachment_id}"))
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

    /// `DELETE /v1/workspaces/{ws}/attachments/{attachment_id}`
    pub async fn delete_attachment(
        &self,
        ws: &str,
        attachment_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/attachments/{attachment_id}"))
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

    /// `PATCH /v1/workspaces/{ws}/documents/{slug}/move`
    pub async fn move_document(
        &self,
        ws: &str,
        slug: &str,
        body: MoveDocumentRequest,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/documents/{slug}/move"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_document").await
    }

    /// `POST /v1/workspaces/{ws}/documents/{slug}/copy`
    pub async fn copy_document(
        &self,
        ws: &str,
        slug: &str,
        folder_id: Option<uuid::Uuid>,
    ) -> Result<DocumentDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/documents/{slug}/copy"))
            .header("x-atlas-csrf", "1")
            .json(&CopyDocumentRequest { folder_id })
            .send()
            .await?;
        self.decode_response(response, "copy_document").await
    }

    // ---- Boards ----------------------------------------------------------------

    /// `POST /v1/workspaces/{ws}/projects/{project_slug}/boards`
    pub async fn create_board(
        &self,
        ws: &str,
        project_slug: &str,
        body: CreateBoardRequest,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/projects/{project_slug}/boards"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_board").await
    }

    /// `GET /v1/workspaces/{ws}/projects/{project_slug}/boards`
    pub async fn list_boards(
        &self,
        ws: &str,
        project_slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<BoardSummaryDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/projects/{project_slug}/boards"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_boards").await
    }

    /// `GET /v1/workspaces/{ws}/boards/{board_id}`
    pub async fn get_board(&self, ws: &str, board_id: uuid::Uuid) -> Result<BoardDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/boards/{board_id}"))
            .send()
            .await?;
        self.decode_response(response, "get_board").await
    }

    /// `PATCH /v1/workspaces/{ws}/boards/{board_id}`
    pub async fn update_board(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: UpdateBoardRequest,
    ) -> Result<BoardDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/boards/{board_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_board").await
    }

    /// `DELETE /v1/workspaces/{ws}/boards/{board_id}`
    pub async fn delete_board(&self, ws: &str, board_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/boards/{board_id}"))
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

    /// `POST /v1/workspaces/{ws}/boards/{board_id}/columns`
    pub async fn create_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: CreateColumnRequest,
    ) -> Result<ColumnDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/boards/{board_id}/columns"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_column").await
    }

    /// `GET /v1/workspaces/{ws}/boards/{board_id}/columns`
    pub async fn list_columns(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
    ) -> Result<Vec<ColumnDto>, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/boards/{board_id}/columns"))
            .send()
            .await?;
        self.decode_response(response, "list_columns").await
    }

    /// `PATCH /v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}`
    pub async fn update_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        column_id: uuid::Uuid,
        body: UpdateColumnRequest,
    ) -> Result<ColumnDto, ClientError> {
        let response = self
            .patch(&format!(
                "/v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_column").await
    }

    /// `DELETE /v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}`
    pub async fn delete_column(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        column_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!(
                "/v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}"
            ))
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

    // ---- Tasks ----------------------------------------------------------------

    /// `POST /v1/workspaces/{ws}/boards/{board_id}/tasks`
    pub async fn create_task(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        body: CreateTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/boards/{board_id}/tasks"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_task").await
    }

    /// `GET /v1/workspaces/{ws}/boards/{board_id}/tasks`
    pub async fn list_tasks(
        &self,
        ws: &str,
        board_id: uuid::Uuid,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<TaskSummaryDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/v1/workspaces/{ws}/boards/{board_id}/tasks"),
            cursor,
            limit,
        );
        let response = self.get(&path).send().await?;
        self.decode_response(response, "list_tasks").await
    }

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}`
    pub async fn get_task(&self, ws: &str, readable_id: &str) -> Result<TaskDto, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/tasks/{readable_id}"))
            .send()
            .await?;
        self.decode_response(response, "get_task").await
    }

    /// `PATCH /v1/workspaces/{ws}/tasks/{readable_id}`
    pub async fn update_task(
        &self,
        ws: &str,
        readable_id: &str,
        body: UpdateTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .patch(&format!("/v1/workspaces/{ws}/tasks/{readable_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_task").await
    }

    /// `DELETE /v1/workspaces/{ws}/tasks/{readable_id}`
    pub async fn delete_task(&self, ws: &str, readable_id: &str) -> Result<(), ClientError> {
        let response = self
            .delete(&format!("/v1/workspaces/{ws}/tasks/{readable_id}"))
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

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/move`
    pub async fn move_task(
        &self,
        ws: &str,
        readable_id: &str,
        body: MoveTaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/tasks/{readable_id}/move"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "move_task").await
    }

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/assignees`
    pub async fn list_assignees(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<AssigneeDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/assignees"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_assignees").await
    }

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/assignees`
    pub async fn add_assignee(
        &self,
        ws: &str,
        readable_id: &str,
        body: AddAssigneeRequest,
    ) -> Result<AssigneeDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/assignees"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "add_assignee").await
    }

    /// `DELETE /v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}`
    pub async fn remove_assignee(
        &self,
        ws: &str,
        readable_id: &str,
        assignee_ref: &str,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}"
            ))
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

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/references`
    pub async fn list_references(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<ReferenceDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/references"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_references").await
    }

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/references`
    pub async fn create_reference(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateReferenceRequest,
    ) -> Result<ReferenceDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/references"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_reference").await
    }

    /// `DELETE /v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}`
    pub async fn delete_reference(
        &self,
        ws: &str,
        readable_id: &str,
        reference_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}"
            ))
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

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/backlinks`
    pub async fn list_task_backlinks(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Page<TaskBacklinkDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/backlinks"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_task_backlinks").await
    }

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/checklist`
    pub async fn list_checklist(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<ChecklistItemDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/checklist"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_checklist").await
    }

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist`
    pub async fn create_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateChecklistItemRequest,
    ) -> Result<ChecklistItemDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/checklist"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_checklist_item")
            .await
    }

    /// `PATCH /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}`
    pub async fn update_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
        body: UpdateChecklistItemRequest,
    ) -> Result<ChecklistItemDto, ClientError> {
        let response = self
            .patch(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_checklist_item")
            .await
    }

    /// `DELETE /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}`
    pub async fn delete_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"
            ))
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

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote`
    pub async fn promote_checklist_item(
        &self,
        ws: &str,
        readable_id: &str,
        item_id: uuid::Uuid,
        body: PromoteChecklistItemRequest,
    ) -> Result<PromotionDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "promote_checklist_item")
            .await
    }

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/subtasks`
    pub async fn list_subtasks(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Vec<TaskSummaryDto>, ClientError> {
        let response = self
            .get(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/subtasks"
            ))
            .send()
            .await?;
        self.decode_response(response, "list_subtasks").await
    }

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/subtasks`
    pub async fn create_subtask(
        &self,
        ws: &str,
        readable_id: &str,
        body: CreateSubtaskRequest,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(&format!(
                "/v1/workspaces/{ws}/tasks/{readable_id}/subtasks"
            ))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_subtask").await
    }

    /// `POST /v1/workspaces/{ws}/tasks/{readable_id}/promote`
    pub async fn promote_subtask(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<TaskDto, ClientError> {
        let response = self
            .post(&format!("/v1/workspaces/{ws}/tasks/{readable_id}/promote"))
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "promote_subtask").await
    }

    /// `GET /v1/workspaces/{ws}/tasks/{readable_id}/activity`
    pub async fn list_activity(
        &self,
        ws: &str,
        readable_id: &str,
    ) -> Result<Page<ActivityEntryDto>, ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/tasks/{readable_id}/activity"))
            .send()
            .await?;
        self.decode_response(response, "list_activity").await
    }

    /// `POST /v1/auth/logout`
    pub async fn logout(&self) -> Result<(), ClientError> {
        let response = self
            .post("/v1/auth/logout")
            .header("x-atlas-csrf", "1")
            .send()
            .await?;

        if !response.status().is_success() {
            let problem: ProblemDetails = response
                .json()
                .await
                .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
            return Err(ClientError::Api(problem));
        }

        Ok(())
    }
}

fn build_search_path(
    ws: &str,
    q: &str,
    type_filter: Option<&str>,
    sort: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    let encoded_q = encode_query_value(q);
    let mut params = vec![format!("q={encoded_q}")];

    if let Some(t) = type_filter {
        params.push(format!("type={t}"));
    }
    if let Some(s) = sort {
        params.push(format!("sort={s}"));
    }
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }

    format!("/v1/workspaces/{ws}/search?{}", params.join("&"))
}

/// Percent-encodes characters that are not safe in a query-string value.
fn hex_nibble(n: u8) -> char {
    char::from_digit(n as u32, 16)
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('0')
}

fn encode_query_value(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf);
                bytes
                    .bytes()
                    .flat_map(|b| vec!['%', hex_nibble(b >> 4), hex_nibble(b & 0x0f)])
                    .collect::<Vec<_>>()
            }
        })
        .collect()
}

fn build_paginated_path(base: &str, cursor: Option<&str>, limit: Option<u32>) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_stores_base_url() {
        let client = AtlasClient::new("http://localhost:8080");
        assert_eq!(client.base_url(), "http://localhost:8080");
    }

    #[test]
    fn with_token_stores_token() {
        let client = AtlasClient::new("http://localhost:8080").with_token("test-token");
        assert!(client.token.is_some());
    }

    #[test]
    fn token_accessor_returns_none_initially() {
        let client = AtlasClient::new("http://localhost:8080");
        assert!(client.token().is_none());
    }

    #[test]
    fn build_search_path_includes_required_q() {
        let path = build_search_path("my-ws", "hello world", None, None, None, None);
        assert!(path.starts_with("/v1/workspaces/my-ws/search?q="));
        assert!(
            path.contains("hello%20world")
                || path.contains("hello+world")
                || path.contains("hello")
        );
    }

    #[test]
    fn build_search_path_includes_optional_params() {
        let path = build_search_path(
            "ws1",
            "query",
            Some("task"),
            Some("updated"),
            Some("abc"),
            Some(10),
        );
        assert!(path.contains("type=task"));
        assert!(path.contains("sort=updated"));
        assert!(path.contains("cursor=abc"));
        assert!(path.contains("limit=10"));
    }

    #[test]
    fn build_search_path_omits_optional_params_when_none() {
        let path = build_search_path("ws1", "query", None, None, None, None);
        assert!(!path.contains("type="));
        assert!(!path.contains("sort="));
        assert!(!path.contains("cursor="));
        assert!(!path.contains("limit="));
    }

    #[test]
    fn encode_query_value_encodes_spaces() {
        let encoded = encode_query_value("hello world");
        assert!(encoded.contains("%20") || !encoded.contains(' '));
    }

    #[test]
    fn encode_query_value_preserves_alphanumeric() {
        let encoded = encode_query_value("abc123");
        assert_eq!(encoded, "abc123");
    }
}
