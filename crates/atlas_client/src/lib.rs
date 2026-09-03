#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod helpers;

use atlas_api::{
    dtos::{
        ActivationLinkResponse, AdminUpdateWorkspaceRequest, ApiKeyCreated, ApiKeyDto,
        ApiKeyGrantDto, ApiKeyScope, ChangePasswordRequest, CreateGrantRequest,
        CreateProjectRequest, CreateUserApiKeyRequest, CreateUserRequest, CreateUserResponse,
        CreateWorkspaceRequest, GrantDto, HealthResponse, LoginRequest, LoginResponse, MeResponse,
        PrincipalDto, ProjectDto, ResetPasswordRequest, ServerMetaDto, UiStateDto, UpdateMeRequest,
        UpdateProjectRequest, UpdateUiStateRequest, UpdateWorkspaceRequest, UserDto,
        UserMembershipDto, WorkspaceDto,
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
        groups::{AddGroupMemberRequest, CreateGroupRequest, GroupDto, GroupMemberDto},
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
use std::time::Duration;
use thiserror::Error;

/// Maximum number of times a request is retried after a 429 before giving up.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
/// Upper bound on how long a single retry waits, regardless of `Retry-After`.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
/// Floor applied to any retry wait so a `Retry-After: 0` still yields a pause.
const MIN_RETRY_WAIT: Duration = Duration::from_millis(50);
/// Total per-request timeout applied to every API call by default.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Time allowed to establish a TCP/TLS connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Relaxed total timeout for attachment uploads/downloads, whose bodies can be
/// large enough that the default request timeout would abort a healthy transfer.
const ATTACHMENT_TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);

/// Renders an RFC 9457 problem as a readable one-line message, preserving the
/// historical `API error {status}: {title}` prefix and appending `detail` and
/// `hint` when the server provided them.
fn api_error_message(problem: &ProblemDetails) -> String {
    let mut message = format!("API error {}: {}", problem.status, problem.title);

    if let Some(detail) = problem.detail.as_deref() {
        message.push_str(&format!(" — {detail}"));
    }
    if let Some(hint) = problem.hint.as_deref() {
        message.push_str(&format!(" (hint: {hint})"));
    }

    message
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{}", api_error_message(.0))]
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

/// Result of a confirmed Trash purge request.
#[derive(Debug, Clone)]
pub enum PurgeTrashResult {
    /// The purge and any required cleanup are complete.
    Complete,
    /// Database deletion committed, while cleanup is still pending or retryable.
    Pending(PurgeStatusDtoResponse),
}

/// A pending request built through one of the verb helpers.
///
/// Delegates the builder methods the client actually uses (`header`, `json`,
/// `body`) and defers the actual send to [`AtlasClient::send_with_retry`], so
/// every request goes through the 429-retry path without changing call sites.
#[must_use = "a Req does nothing until `.send().await` is awaited"]
struct Req<'a> {
    client: &'a AtlasClient,
    builder: reqwest::RequestBuilder,
}

impl Req<'_> {
    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.builder = self.builder.header(name, value.into());
        self
    }

    fn json<T: serde::Serialize + ?Sized>(mut self, json: &T) -> Self {
        self.builder = self.builder.json(json);
        self
    }

    fn body(mut self, body: impl Into<reqwest::Body>) -> Self {
        self.builder = self.builder.body(body);
        self
    }

    /// Overrides the client-level total timeout for this single request.
    fn timeout(mut self, timeout: Duration) -> Self {
        self.builder = self.builder.timeout(timeout);
        self
    }

    async fn send(self) -> Result<reqwest::Response, ClientError> {
        self.client.send_with_retry(self.builder).await
    }
}

/// Parses a `Retry-After` header value (delta-seconds) into a bounded wait.
///
/// The Atlas server emits an integer number of seconds. Anything missing or
/// unparseable falls back to one second. The result is clamped to
/// `[MIN_RETRY_WAIT, MAX_RETRY_WAIT]` so a hostile or misconfigured value cannot
/// make the client sleep indefinitely or busy-loop.
fn parse_retry_after(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(1);

    Duration::from_secs(secs).clamp(MIN_RETRY_WAIT, MAX_RETRY_WAIT)
}

/// The three REG-5 components whose routes this client addresses. The
/// variants' wire strings are the `/api/v2/<component>` mount segments
/// (`atlas_server::lib::app`); `atlas_server`'s route-contract test asserts
/// they are exactly the registry's API-declaring stable ids, so this enum
/// cannot drift from ownership even though this crate cannot see the
/// registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Platform,
    Custos,
    Acta,
}

impl Component {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Custos => "custos",
            Self::Acta => "acta",
        }
    }
}

// api-path-guard:off
const V2_PREFIX: &str = "/api/v2";
// api-path-guard:on

pub struct AtlasClient {
    base_url: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl AtlasClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        // Building with timeouts can only fail on TLS/resolver misconfiguration;
        // fall back to the default client rather than change the signature.
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            base_url: base_url.into(),
            http,
            token: None,
        }
    }

    /// Constructs a client that reuses an existing `reqwest::Client` connection pool.
    ///
    /// `reqwest::Client` is Arc-backed internally, so cloning it is cheap — this
    /// constructor takes ownership of the caller's clone and avoids spawning a new
    /// DNS resolver or TLS stack for each logical atlas_mcp session.
    pub fn with_shared_pool(
        pool: reqwest::Client,
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            http: pool,
            token: Some(token.into()),
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

    /// Resolves `component`'s `/api/v2/<component>` mount plus `relative`
    /// into an absolute URL. The only place a component namespace is
    /// assembled into a concrete URL inside this crate.
    fn mounted(&self, component: Component, relative: &str) -> String {
        format!(
            "{}{V2_PREFIX}/{}{relative}",
            self.base_url,
            component.as_str()
        )
    }

    fn get(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.get(self.mounted(component, relative)))
    }

    fn post(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.post(self.mounted(component, relative)))
    }

    fn patch(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.patch(self.mounted(component, relative)))
    }

    fn put(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.put(self.mounted(component, relative)))
    }

    fn delete(&self, component: Component, relative: &str) -> Req<'_> {
        self.request(self.http.delete(self.mounted(component, relative)))
    }

    /// The one composition-root route this client calls outside every
    /// component nest (`router_audit::ROOT_LEVEL_PATHS`): `GET /health`.
    /// Kept as a distinct method rather than a `Component` argument because a
    /// root-level path has no owning namespace on the wire.
    fn root_get(&self, path: &str) -> Req<'_> {
        self.request(self.http.get(format!("{}{}", self.base_url, path)))
    }

    /// Wraps a raw `RequestBuilder`, applying bearer auth, into a retry-aware `Req`.
    fn request(&self, mut builder: reqwest::RequestBuilder) -> Req<'_> {
        if let Some(token) = &self.token {
            builder = builder.bearer_auth(token);
        }
        Req {
            client: self,
            builder,
        }
    }

    /// Sends a request, transparently retrying on HTTP 429.
    ///
    /// On a `429 Too Many Requests` the server's per-principal rate limit was hit;
    /// the response carries a `Retry-After` interval. Bulk callers (the CLI and
    /// MCP server) would otherwise fail on the first rejection, so the client
    /// waits for that interval and retries up to `MAX_RATE_LIMIT_RETRIES` times.
    ///
    /// Requests whose body cannot be cloned (streaming bodies) are sent once with
    /// no retry, since replaying them is not possible.
    async fn send_with_retry(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let mut attempt: u32 = 0;

        loop {
            let attempt_builder = match request.try_clone() {
                Some(clone) => clone,
                None => return Ok(request.send().await?),
            };

            let response = attempt_builder.send().await?;

            if response.status().as_u16() == 429 && attempt < MAX_RATE_LIMIT_RETRIES {
                let wait = parse_retry_after(
                    response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok()),
                );
                attempt += 1;
                tokio::time::sleep(wait).await;
                continue;
            }

            return Ok(response);
        }
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
        let response = self.root_get("/health").send().await?;
        self.decode_response(response, "health").await
    }

    /// `POST /api/v2/custos/auth/login`
    ///
    /// On success, stores the returned session token in `self.token`.
    pub async fn login(&mut self, body: LoginRequest) -> Result<LoginResponse, ClientError> {
        let response = self
            .post(Component::Custos, "/auth/login")
            .json(&body)
            .send()
            .await?;
        let login: LoginResponse = self.decode_response(response, "login").await?;
        self.token = Some(login.token.clone());
        Ok(login)
    }

    /// `GET /api/v2/custos/auth/me`
    pub async fn me(&self) -> Result<MeResponse, ClientError> {
        let response = self.get(Component::Custos, "/auth/me").send().await?;
        self.decode_response(response, "me").await
    }

    /// `POST /api/v2/custos/auth/change-password`
    pub async fn change_password(&self, body: ChangePasswordRequest) -> Result<(), ClientError> {
        let response = self
            .post(Component::Custos, "/auth/change-password")
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

    /// `PATCH /api/v2/custos/users/me`
    pub async fn update_me(&self, body: UpdateMeRequest) -> Result<UserDto, ClientError> {
        let response = self
            .patch(Component::Custos, "/users/me")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "update_me").await
    }

    /// `GET /api/v2/platform/me/ui-state`
    ///
    /// Returns the current user's stored UI state object (an empty object when
    /// no state has been saved yet).
    pub async fn get_ui_state(&self) -> Result<serde_json::Value, ClientError> {
        let response = self.get(Component::Platform, "/me/ui-state").send().await?;
        let dto: UiStateDto = self.decode_response(response, "get_ui_state").await?;
        Ok(dto.state)
    }

    /// `PUT /api/v2/platform/me/ui-state`
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
            .put(Component::Platform, "/me/ui-state")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        let dto: UiStateDto = self.decode_response(response, "set_ui_state").await?;
        Ok(dto.state)
    }

    /// `GET /api/v2/platform/meta`
    pub async fn server_meta(&self) -> Result<ServerMetaDto, ClientError> {
        let response = self.get(Component::Platform, "/meta").send().await?;
        self.decode_response(response, "server_meta").await
    }

    /// `GET /api/v2/custos/users`
    pub async fn list_users(&self) -> Result<Vec<UserDto>, ClientError> {
        let response = self.get(Component::Custos, "/users").send().await?;
        self.decode_response(response, "list_users").await
    }

    /// `POST /api/v2/custos/users`
    pub async fn create_user(
        &self,
        body: CreateUserRequest,
    ) -> Result<CreateUserResponse, ClientError> {
        let response = self
            .post(Component::Custos, "/users")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_user").await
    }

    /// `POST /api/v2/custos/users/{user_id}/activation-link`
    pub async fn regenerate_activation_link(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<ActivationLinkResponse, ClientError> {
        let response = self
            .post(
                Component::Custos,
                &format!("/users/{user_id}/activation-link"),
            )
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "regenerate_activation_link")
            .await
    }

    /// `POST /api/v2/custos/users/{user_id}/disable`
    pub async fn disable_user(&self, user_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .post(Component::Custos, &format!("/users/{user_id}/disable"))
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

    /// `POST /api/v2/custos/users/{user_id}/enable`
    pub async fn enable_user(&self, user_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .post(Component::Custos, &format!("/users/{user_id}/enable"))
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

    /// `POST /api/v2/custos/users/{user_id}/reset-password`
    pub async fn reset_user_password(
        &self,
        user_id: uuid::Uuid,
        new_password: impl Into<String>,
    ) -> Result<(), ClientError> {
        let response = self
            .post(
                Component::Custos,
                &format!("/users/{user_id}/reset-password"),
            )
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

    /// `GET /api/v2/custos/users/{user_id}/memberships`
    ///
    /// Lists every workspace the target user belongs to, with the membership
    /// role. Requires root/admin privileges.
    pub async fn list_user_memberships(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Vec<UserMembershipDto>, ClientError> {
        let response = self
            .get(Component::Custos, &format!("/users/{user_id}/memberships"))
            .send()
            .await?;
        self.decode_response(response, "list_user_memberships")
            .await
    }

    /// `POST /api/v2/custos/api-keys`
    pub async fn create_user_api_key(
        &self,
        body: CreateUserApiKeyRequest,
    ) -> Result<ApiKeyCreated, ClientError> {
        let response = self
            .post(Component::Custos, "/api-keys")
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_user_api_key").await
    }

    /// `GET /api/v2/custos/api-keys`
    pub async fn list_user_api_keys(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<ApiKeyDto>, ClientError> {
        let path = build_paginated_path("/api-keys", cursor, limit);
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_user_api_keys").await
    }

    /// `DELETE /api/v2/custos/api-keys/{key_id}`
    pub async fn revoke_user_api_key(&self, key_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(Component::Custos, &format!("/api-keys/{key_id}"))
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

    /// `PATCH /api/v2/custos/api-keys/{key_id}`
    pub async fn set_api_key_global(
        &self,
        key_id: uuid::Uuid,
        is_global: bool,
    ) -> Result<ApiKeyDto, ClientError> {
        use atlas_api::dtos::UpdateApiKeyRequest;

        let body = UpdateApiKeyRequest {
            is_global: Some(is_global),
            scopes: None,
        };
        let response = self
            .patch(Component::Custos, &format!("/api-keys/{key_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "set_api_key_global").await
    }

    /// `PATCH /api/v2/custos/api-keys/{key_id}`
    pub async fn set_api_key_scopes(
        &self,
        key_id: uuid::Uuid,
        scopes: Vec<ApiKeyScope>,
    ) -> Result<ApiKeyDto, ClientError> {
        use atlas_api::dtos::UpdateApiKeyRequest;

        let body = UpdateApiKeyRequest {
            is_global: None,
            scopes: Some(scopes),
        };
        let response = self
            .patch(Component::Custos, &format!("/api-keys/{key_id}"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "set_api_key_scopes").await
    }

    /// `GET /api/v2/custos/api-keys/{key_id}/grants`
    pub async fn list_api_key_grants(
        &self,
        key_id: uuid::Uuid,
    ) -> Result<Vec<ApiKeyGrantDto>, ClientError> {
        let response = self
            .get(Component::Custos, &format!("/api-keys/{key_id}/grants"))
            .send()
            .await?;
        self.decode_response(response, "list_api_key_grants").await
    }

    /// `DELETE /api/v2/custos/api-keys/{key_id}/grants/{grant_id}`
    pub async fn delete_api_key_grant(
        &self,
        key_id: uuid::Uuid,
        grant_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Custos,
                &format!("/api-keys/{key_id}/grants/{grant_id}"),
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

    /// `POST /api/v2/custos/workspaces/{ws}/projects/{slug}/grants`
    pub async fn create_project_grant(
        &self,
        ws: &str,
        slug: &str,
        body: CreateGrantRequest,
    ) -> Result<GrantDto, ClientError> {
        let response = self
            .post(
                Component::Custos,
                &format!("/workspaces/{ws}/projects/{slug}/grants"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_project_grant").await
    }

    /// `GET /api/v2/custos/workspaces/{ws}/projects/{slug}/grants`
    pub async fn list_project_grants(
        &self,
        ws: &str,
        slug: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<GrantDto>, ClientError> {
        let path = build_paginated_path(
            &format!("/workspaces/{ws}/projects/{slug}/grants"),
            cursor,
            limit,
        );
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_project_grants").await
    }

    /// `DELETE /api/v2/custos/workspaces/{ws}/projects/{slug}/grants/{grant_id}`
    pub async fn delete_project_grant(
        &self,
        ws: &str,
        slug: &str,
        grant_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Custos,
                &format!("/workspaces/{ws}/projects/{slug}/grants/{grant_id}"),
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

    /// `POST /api/v2/custos/workspaces/{ws}/grants`
    pub async fn create_workspace_grant(
        &self,
        ws: &str,
        body: CreateGrantRequest,
    ) -> Result<GrantDto, ClientError> {
        let response = self
            .post(Component::Custos, &format!("/workspaces/{ws}/grants"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_workspace_grant")
            .await
    }

    /// `GET /api/v2/custos/workspaces/{ws}/grants`
    pub async fn list_workspace_grants(
        &self,
        ws: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<GrantDto>, ClientError> {
        let path = build_paginated_path(&format!("/workspaces/{ws}/grants"), cursor, limit);
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_workspace_grants")
            .await
    }

    /// `DELETE /api/v2/custos/workspaces/{ws}/grants/{grant_id}`
    pub async fn delete_workspace_grant(
        &self,
        ws: &str,
        grant_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Custos,
                &format!("/workspaces/{ws}/grants/{grant_id}"),
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

    // ---- Admin Trash -----------------------------------------------------------

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

    // ---- Webhooks --------------------------------------------------------------

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

    // ---- Boards ----------------------------------------------------------------

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

    // ---- Status templates -------------------------------------------------------

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

    // ---- Platform status templates (Atlas-wide defaults, admin-only) -------------

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

    // ---- Tasks ----------------------------------------------------------------

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

    /// `GET /api/v2/custos/workspaces/{ws}/audit`
    pub async fn list_workspace_audit(
        &self,
        ws: &str,
        actor: Option<&str>,
        action: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<atlas_api::dtos::audit::AuditEntryDto>, ClientError> {
        let path = build_audit_path(
            &format!("/workspaces/{ws}/audit"),
            actor,
            action,
            from,
            to,
            None,
            limit,
        );
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_workspace_audit").await
    }

    /// `GET /api/v2/custos/workspaces/{ws}/audit`
    pub async fn list_workspace_audit_with_cursor(
        &self,
        ws: &str,
        actor: Option<&str>,
        action: Option<&str>,
        from: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<atlas_api::dtos::audit::AuditEntryDto>, ClientError> {
        let path = build_audit_path(
            &format!("/workspaces/{ws}/audit"),
            actor,
            action,
            from,
            None,
            cursor,
            limit,
        );
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_workspace_audit_with_cursor")
            .await
    }

    /// `GET /api/v2/custos/admin/audit`
    pub async fn list_platform_audit(
        &self,
        actor: Option<&str>,
        action: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<atlas_api::dtos::audit::AuditEntryDto>, ClientError> {
        let path = build_audit_path("/admin/audit", actor, action, from, to, None, limit);
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_platform_audit").await
    }

    /// `GET /api/v2/custos/admin/audit`
    pub async fn list_platform_audit_with_cursor(
        &self,
        actor: Option<&str>,
        action: Option<&str>,
        from: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<atlas_api::dtos::audit::AuditEntryDto>, ClientError> {
        let path = build_audit_path("/admin/audit", actor, action, from, None, cursor, limit);
        let response = self.get(Component::Custos, &path).send().await?;
        self.decode_response(response, "list_platform_audit_with_cursor")
            .await
    }

    /// `POST /api/v2/custos/auth/logout`
    pub async fn logout(&self) -> Result<(), ClientError> {
        let response = self
            .post(Component::Custos, "/auth/logout")
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

    // ---- Groups ----------------------------------------------------------------

    /// `POST /api/v2/custos/workspaces/{ws}/groups`
    pub async fn create_group(
        &self,
        ws: &str,
        body: CreateGroupRequest,
    ) -> Result<GroupDto, ClientError> {
        let response = self
            .post(Component::Custos, &format!("/workspaces/{ws}/groups"))
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "create_group").await
    }

    /// `GET /api/v2/custos/workspaces/{ws}/groups`
    pub async fn list_groups(&self, ws: &str) -> Result<Vec<GroupDto>, ClientError> {
        let response = self
            .get(Component::Custos, &format!("/workspaces/{ws}/groups"))
            .send()
            .await?;
        self.decode_response(response, "list_groups").await
    }

    /// `DELETE /api/v2/custos/workspaces/{ws}/groups/{group_id}`
    pub async fn delete_group(&self, ws: &str, group_id: uuid::Uuid) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Custos,
                &format!("/workspaces/{ws}/groups/{group_id}"),
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

    /// `POST /api/v2/custos/workspaces/{ws}/groups/{group_id}/members`
    pub async fn add_group_member(
        &self,
        ws: &str,
        group_id: uuid::Uuid,
        body: AddGroupMemberRequest,
    ) -> Result<GroupMemberDto, ClientError> {
        let response = self
            .post(
                Component::Custos,
                &format!("/workspaces/{ws}/groups/{group_id}/members"),
            )
            .header("x-atlas-csrf", "1")
            .json(&body)
            .send()
            .await?;
        self.decode_response(response, "add_group_member").await
    }

    /// `DELETE /api/v2/custos/workspaces/{ws}/groups/{group_id}/members/{user_id}`
    pub async fn remove_group_member(
        &self,
        ws: &str,
        group_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ClientError> {
        let response = self
            .delete(
                Component::Custos,
                &format!("/workspaces/{ws}/groups/{group_id}/members/{user_id}"),
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

    /// `GET /api/v2/custos/workspaces/{ws}/groups/{group_id}/members`
    pub async fn list_group_members(
        &self,
        ws: &str,
        group_id: uuid::Uuid,
    ) -> Result<Vec<GroupMemberDto>, ClientError> {
        let response = self
            .get(
                Component::Custos,
                &format!("/workspaces/{ws}/groups/{group_id}/members"),
            )
            .send()
            .await?;
        self.decode_response(response, "list_group_members").await
    }
}

fn build_search_path(
    ws: &str,
    q: &str,
    type_filter: Option<&str>,
    sort: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
    mode: Option<&str>,
) -> String {
    let mut params = search_params(q, type_filter, cursor, limit);
    if let Some(s) = sort {
        params.insert(2.min(params.len()), format!("sort={s}"));
    }
    if let Some(mode) = mode {
        params.push(format!("mode={mode}"));
    }

    format!("/workspaces/{ws}/search?{}", params.join("&"))
}

fn build_document_range_path(ws: &str, slug: &str, query: &DocumentContentRangeQuery) -> String {
    let base = format!("/workspaces/{ws}/documents/{slug}/content/range");
    let mut params = Vec::new();

    if let Some(start_line) = query.start_line {
        params.push(format!("start_line={start_line}"));
    }
    if let Some(end_line) = query.end_line {
        params.push(format!("end_line={end_line}"));
    }
    if let Some(line_limit) = query.line_limit {
        params.push(format!("line_limit={line_limit}"));
    }
    if let Some(byte_limit) = query.byte_limit {
        params.push(format!("byte_limit={byte_limit}"));
    }
    if let Some(continuation) = query.continuation.as_deref() {
        params.push(format!("continuation={}", encode_query_value(continuation)));
    }

    if params.is_empty() {
        base
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

fn build_semantic_search_path(
    ws: &str,
    q: &str,
    type_filter: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    format!(
        "/workspaces/{ws}/semantic-search?{}",
        search_params(q, type_filter, cursor, limit).join("&")
    )
}

fn search_params(
    q: &str,
    type_filter: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Vec<String> {
    let encoded_q = encode_query_value(q);
    let mut params = vec![format!("q={encoded_q}")];

    if let Some(t) = type_filter {
        params.push(format!("type={t}"));
    }
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    params
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

fn build_workspace_tasks_path(ws: &str, q: &WorkspaceTaskQueryParams) -> String {
    let base = format!("/workspaces/{ws}/tasks");
    let mut params: Vec<String> = Vec::new();

    if let Some(a) = &q.assignee {
        params.push(format!("assignee={a}"));
    }
    if let Some(a) = &q.actor {
        params.push(format!("actor={a}"));
    }
    for col in &q.column_ids {
        params.push(format!("column_id={col}"));
    }
    for pri in &q.priorities {
        params.push(format!("priority={pri}"));
    }
    for lbl in &q.labels {
        params.push(format!("label={lbl}"));
    }
    if let Some(b) = &q.board_id {
        params.push(format!("board_id={b}"));
    }
    if let Some(s) = &q.sort {
        params.push(format!("sort={s}"));
    }
    if let Some(c) = &q.cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = q.limit {
        params.push(format!("limit={l}"));
    }

    if params.is_empty() {
        base
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

fn build_audit_path(
    base: &str,
    actor: Option<&str>,
    action: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(a) = actor {
        params.push(format!("actor={a}"));
    }
    if let Some(a) = action {
        params.push(format!("action={}", encode_query_value(a)));
    }
    if let Some(f) = from {
        params.push(format!("from={}", encode_query_value(f)));
    }
    if let Some(t) = to {
        params.push(format!("to={}", encode_query_value(t)));
    }
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

fn build_workspace_activity_path(
    ws: &str,
    actor: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    let base = format!("/workspaces/{ws}/activity");
    let mut params: Vec<String> = Vec::new();
    if let Some(a) = actor {
        params.push(format!("actor={a}"));
    }
    if let Some(f) = from {
        params.push(format!("from={}", encode_query_value(f)));
    }
    if let Some(t) = to {
        params.push(format!("to={}", encode_query_value(t)));
    }
    if let Some(c) = cursor {
        params.push(format!("cursor={c}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    if params.is_empty() {
        base
    } else {
        format!("{}?{}", base, params.join("&"))
    }
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

fn build_document_list_path(
    base: &str,
    cursor: Option<&str>,
    limit: Option<u32>,
    unfiled: Option<bool>,
    preview: bool,
) -> String {
    let mut path = build_paginated_path(base, cursor, limit);

    if let Some(value) = unfiled {
        path = append_query_flag(path, "unfiled", value);
    }
    if preview {
        path = append_query_flag(path, "preview", true);
    }

    path
}

fn append_query_flag(path: String, name: &str, value: bool) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };

    format!("{path}{separator}{name}={value}")
}

fn build_trash_list_path(
    workspace_id: Option<uuid::Uuid>,
    kind: Option<TrashKindDto>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> String {
    let mut params = Vec::new();

    if let Some(workspace_id) = workspace_id {
        params.push(format!("workspace_id={workspace_id}"));
    }
    if let Some(kind) = kind {
        params.push(format!("kind={}", trash_kind_query(kind)));
    }
    if let Some(cursor) = cursor {
        params.push(format!("cursor={}", encode_query_value(cursor)));
    }
    if let Some(limit) = limit {
        params.push(format!("limit={limit}"));
    }

    if params.is_empty() {
        "/admin/trash".to_string()
    } else {
        format!("/admin/trash?{}", params.join("&"))
    }
}

fn trash_kind_query(kind: TrashKindDto) -> &'static str {
    match kind {
        TrashKindDto::Project => "project",
        TrashKindDto::Folder => "folder",
        TrashKindDto::Document => "document",
        TrashKindDto::Comment => "comment",
        TrashKindDto::Attachment => "attachment",
    }
}

fn build_comment_feed_path(base: &str, cursor: Option<&str>, limit: Option<u32>) -> String {
    let mut params = vec!["feed=full".to_string()];
    if let Some(cursor) = cursor {
        params.push(format!("cursor={cursor}"));
    }
    if let Some(limit) = limit {
        params.push(format!("limit={limit}"));
    }
    format!("{}?{}", base, params.join("&"))
}

async fn decode_empty_response(response: reqwest::Response) -> Result<(), ClientError> {
    if response.status().is_success() {
        return Ok(());
    }
    let problem = response
        .json()
        .await
        .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
    Err(ClientError::Api(problem))
}

async fn decode_attachment_content(
    response: reqwest::Response,
) -> Result<(Vec<u8>, Option<String>), ClientError> {
    if !response.status().is_success() {
        let problem = response
            .json()
            .await
            .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
        return Err(ClientError::Api(problem));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let bytes = response.bytes().await?;
    Ok((bytes.to_vec(), content_type))
}

/// Builds the `GET /api/v2/acta/workspaces/{ws}/webhooks` path.
///
/// The webhook list route paginates on `after` (forward cursor) rather than the
/// generic `cursor` param, so it cannot reuse [`build_paginated_path`].
fn build_webhooks_list_path(ws: &str, after: Option<&str>, limit: Option<u32>) -> String {
    let base = format!("/workspaces/{ws}/webhooks");

    let mut params: Vec<String> = Vec::new();
    if let Some(a) = after {
        params.push(format!("after={a}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }

    if params.is_empty() {
        base
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

/// Builds the `GET /api/v2/acta/workspaces/{ws}/webhooks/{webhook_id}/deliveries` path.
///
/// Delivery attempts paginate newest-first on `before`, so this route also
/// cannot reuse [`build_paginated_path`].
fn build_webhook_deliveries_path(
    ws: &str,
    webhook_id: uuid::Uuid,
    before: Option<&str>,
    limit: Option<u32>,
) -> String {
    let base = format!("/workspaces/{ws}/webhooks/{webhook_id}/deliveries");

    let mut params: Vec<String> = Vec::new();
    if let Some(b) = before {
        params.push(format!("before={b}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }

    if params.is_empty() {
        base
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_api::dtos::lifecycle::PurgeStatusDto;

    #[test]
    fn construction_stores_base_url() {
        let client = AtlasClient::new("http://localhost:8080");
        assert_eq!(client.base_url(), "http://localhost:8080");
    }

    #[test]
    fn component_as_str_matches_the_registrys_stable_ids() {
        assert_eq!(Component::Platform.as_str(), "platform");
        assert_eq!(Component::Custos.as_str(), "custos");
        assert_eq!(Component::Acta.as_str(), "acta");
    }

    #[test]
    fn mounted_joins_base_url_v2_prefix_component_and_relative_path() {
        let client = AtlasClient::new("http://localhost:8080");
        assert_eq!(
            client.mounted(Component::Acta, "/workspaces/{ws}/tasks"),
            "http://localhost:8080/api/v2/acta/workspaces/{ws}/tasks"
        );
        assert_eq!(
            client.mounted(Component::Custos, "/users"),
            "http://localhost:8080/api/v2/custos/users"
        );
        assert_eq!(
            client.mounted(Component::Platform, "/meta"),
            "http://localhost:8080/api/v2/platform/meta"
        );
    }

    #[tokio::test]
    async fn verb_methods_resolve_addresses_through_the_component_seam() {
        let (base_url, requests) = serve_once_observing("200 OK", "{}");
        let client = AtlasClient::new(base_url);
        let _ = client.get(Component::Acta, "/x").send().await;
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with("GET /api/v2/acta/x "),
            "expected V2-mounted request, got `{raw}`"
        );
    }

    #[tokio::test]
    async fn root_get_addresses_the_root_level_mount_with_no_component_or_api_prefix() {
        let (base_url, requests) = serve_once_observing("200 OK", "{}");
        let client = AtlasClient::new(base_url);
        let _ = client.root_get("/health").send().await;
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with("GET /health "),
            "expected an unprefixed root-level request, got `{raw}`"
        );
    }

    /// D2.6 — one runtime proof of the seam: five methods (one per verb),
    /// proving the seam mechanism rather than enumerating the client's
    /// surface (the source-walking contract test in `atlas_server` owns
    /// that).
    #[tokio::test]
    async fn every_verb_resolves_its_predicted_v2_mounted_url() {
        for (verb, expected_line) in [
            ("get", "GET /api/v2/acta/x "),
            ("post", "POST /api/v2/custos/y "),
            ("patch", "PATCH /api/v2/platform/z "),
            ("put", "PUT /api/v2/acta/w "),
            ("delete", "DELETE /api/v2/custos/q "),
        ] {
            let (base_url, requests) = serve_once_observing("200 OK", "{}");
            let client = AtlasClient::new(base_url);
            let _ = match verb {
                "get" => client.get(Component::Acta, "/x").send().await,
                "post" => client.post(Component::Custos, "/y").send().await,
                "patch" => client.patch(Component::Platform, "/z").send().await,
                "put" => client.put(Component::Acta, "/w").send().await,
                "delete" => client.delete(Component::Custos, "/q").send().await,
                _ => unreachable!(),
            };
            let raw = requests.recv().expect("mock server received request");
            assert!(
                raw.starts_with(expected_line),
                "verb `{verb}`: expected `{expected_line}`, got `{raw}`"
            );
        }
    }

    #[test]
    fn api_error_display_keeps_status_and_title_prefix() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422);
        let message = ClientError::Api(problem).to_string();
        assert_eq!(message, "API error 422: Invalid input");
    }

    #[test]
    fn api_error_display_includes_detail_and_hint_when_present() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422)
            .with_detail("title must not be empty")
            .with_hint("provide a non-empty title");
        let message = ClientError::Api(problem).to_string();
        assert_eq!(
            message,
            "API error 422: Invalid input — title must not be empty \
             (hint: provide a non-empty title)"
        );
    }

    #[test]
    fn api_error_display_includes_detail_without_hint() {
        let problem = ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422)
            .with_detail("title must not be empty");
        let message = ClientError::Api(problem).to_string();
        assert_eq!(
            message,
            "API error 422: Invalid input — title must not be empty"
        );
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
        let path = build_search_path("my-ws", "hello world", None, None, None, None, None);
        assert!(path.starts_with("/workspaces/my-ws/search?q="));
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
            Some("hybrid"),
        );
        assert!(path.contains("type=task"));
        assert!(path.contains("sort=updated"));
        assert!(path.contains("cursor=abc"));
        assert!(path.contains("limit=10"));
        assert!(path.contains("mode=hybrid"));
    }

    #[test]
    fn build_search_path_omits_optional_params_when_none() {
        let path = build_search_path("ws1", "query", None, None, None, None, None);
        assert!(!path.contains("type="));
        assert!(!path.contains("sort="));
        assert!(!path.contains("cursor="));
        assert!(!path.contains("limit="));
        assert!(!path.contains("mode="));
    }

    #[test]
    fn build_document_list_path_omits_unfiled_when_filter_is_any() {
        let path = build_document_list_path("/documents", Some("cursor"), Some(20), None, false);
        assert_eq!(path, "/documents?cursor=cursor&limit=20");
    }

    #[test]
    fn build_document_list_path_keeps_true_and_false_unfiled_states() {
        let unfiled = build_document_list_path("/documents", None, None, Some(true), false);
        let filed = build_document_list_path("/documents", None, None, Some(false), false);

        assert_eq!(unfiled, "/documents?unfiled=true");
        assert_eq!(filed, "/documents?unfiled=false");
    }

    #[test]
    fn build_document_list_path_appends_preview_only_when_opted_in() {
        let without = build_document_list_path("/documents", None, None, None, false);
        let with = build_document_list_path("/documents", None, None, None, true);
        let combined = build_document_list_path("/documents", None, Some(10), Some(true), true);

        assert_eq!(without, "/documents");
        assert_eq!(with, "/documents?preview=true");
        assert_eq!(combined, "/documents?limit=10&unfiled=true&preview=true");
    }

    #[test]
    fn build_semantic_search_path_targets_separate_route() {
        let path = build_semantic_search_path(
            "ws1",
            "concept drift",
            Some("document"),
            Some("cur"),
            Some(25),
        );
        assert!(path.starts_with("/workspaces/ws1/semantic-search?q="));
        assert!(!path.starts_with("/workspaces/ws1/search"));
        assert!(path.contains("concept%20drift"));
        assert!(path.contains("type=document"));
        assert!(path.contains("cursor=cur"));
        assert!(path.contains("limit=25"));
    }

    #[test]
    fn build_semantic_search_path_omits_optional_params_when_none() {
        let path = build_semantic_search_path("ws1", "query", None, None, None);
        assert_eq!(path, "/workspaces/ws1/semantic-search?q=query");
    }

    #[test]
    fn build_webhooks_list_path_uses_after_cursor() {
        let path = build_webhooks_list_path("ws1", Some("cur0"), Some(25));
        assert_eq!(path, "/workspaces/ws1/webhooks?after=cur0&limit=25");
    }

    #[test]
    fn build_webhooks_list_path_omits_params_when_none() {
        let path = build_webhooks_list_path("ws1", None, None);
        assert_eq!(path, "/workspaces/ws1/webhooks");
        assert!(!path.contains("cursor="));
        assert!(!path.contains("after="));
    }

    #[test]
    fn build_webhook_deliveries_path_uses_before_cursor() {
        let id = uuid::Uuid::nil();
        let path = build_webhook_deliveries_path("ws1", id, Some("cur9"), Some(10));
        assert_eq!(
            path,
            format!("/workspaces/ws1/webhooks/{id}/deliveries?before=cur9&limit=10")
        );
    }

    #[test]
    fn build_webhook_deliveries_path_omits_params_when_none() {
        let id = uuid::Uuid::nil();
        let path = build_webhook_deliveries_path("ws1", id, None, None);
        assert_eq!(path, format!("/workspaces/ws1/webhooks/{id}/deliveries"));
        assert!(!path.contains("before="));
    }

    #[test]
    fn build_comment_feed_path_keeps_feed_before_pagination() {
        assert_eq!(
            build_comment_feed_path(
                "/workspaces/ws/tasks/ATL-1/comments",
                Some("cursor"),
                Some(25)
            ),
            "/workspaces/ws/tasks/ATL-1/comments?feed=full&cursor=cursor&limit=25"
        );
    }

    #[test]
    fn build_comment_feed_path_always_requests_full_feed() {
        assert_eq!(
            build_comment_feed_path("/workspaces/ws/documents/doc/comments", None, None),
            "/workspaces/ws/documents/doc/comments?feed=full"
        );
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

    #[test]
    fn parse_retry_after_reads_delta_seconds() {
        assert_eq!(parse_retry_after(Some("5")), Duration::from_secs(5));
        assert_eq!(parse_retry_after(Some("  3 ")), Duration::from_secs(3));
    }

    #[test]
    fn parse_retry_after_defaults_to_one_second_when_absent_or_invalid() {
        assert_eq!(parse_retry_after(None), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("soon")), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("")), Duration::from_secs(1));
    }

    #[test]
    fn parse_retry_after_clamps_to_bounds() {
        assert_eq!(parse_retry_after(Some("0")), MIN_RETRY_WAIT);
        assert_eq!(parse_retry_after(Some("9999")), MAX_RETRY_WAIT);
    }

    fn serve_once(status: &'static str, body: &'static str) -> String {
        serve_once_observing(status, body).0
    }

    fn serve_once_observing(
        status: &'static str,
        body: impl Into<String> + Send + 'static,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let body = body.into();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let length = stream.read(&mut request).unwrap();
            let request = request.get(..length).unwrap_or_default();
            let _ = request_tx.send(String::from_utf8_lossy(request).into_owned());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}"), request_rx)
    }

    async fn assert_comment_attachment_request<T>(
        requests: std::sync::mpsc::Receiver<String>,
        expected_prefix: &str,
        request: impl std::future::Future<Output = Result<T, ClientError>>,
    ) -> T {
        let result = request.await.expect("client lifecycle request succeeds");
        let raw = requests.recv().expect("mock server received request");
        assert!(
            raw.starts_with(expected_prefix),
            "expected `{expected_prefix}`, received `{raw}`"
        );
        result
    }

    async fn assert_comment_attachment_request_with_headers<T>(
        requests: std::sync::mpsc::Receiver<String>,
        expected_prefix: &str,
        expected_headers: &[&str],
        request: impl std::future::Future<Output = Result<T, ClientError>>,
    ) -> T {
        let result = request.await.expect("client lifecycle request succeeds");
        let raw = requests.recv().expect("mock server received request");

        assert!(
            raw.starts_with(expected_prefix),
            "expected `{expected_prefix}`, received `{raw}`"
        );

        for expected_header in expected_headers {
            assert!(
                raw.contains(expected_header),
                "expected request to contain `{expected_header}`, received `{raw}`"
            );
        }

        result
    }

    #[tokio::test]
    async fn server_meta_preserves_optional_limit_and_decode_context() {
        let absent = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null}"#,
        ));
        let absent_meta = absent.server_meta().await.unwrap();
        assert!(absent_meta.max_attachment_bytes.is_none());
        assert!(absent_meta.semantic_search_enabled.is_none());

        let null = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":null}"#,
        ));
        assert!(
            null.server_meta()
                .await
                .unwrap()
                .max_attachment_bytes
                .is_none()
        );

        let disabled = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"semantic_search_enabled":false}"#,
        ));
        assert_eq!(
            disabled
                .server_meta()
                .await
                .unwrap()
                .semantic_search_enabled,
            Some(false)
        );

        let malformed = AtlasClient::new(serve_once(
            "200 OK",
            r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":"large"}"#,
        ));
        assert!(matches!(
            malformed.server_meta().await,
            Err(ClientError::Decode {
                context: "server_meta",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn server_meta_preserves_api_and_transport_errors() {
        let api = AtlasClient::new(serve_once(
            "503 Service Unavailable",
            r#"{"type":"urn:atlas:error","title":"Unavailable","status":503}"#,
        ));
        assert!(matches!(api.server_meta().await, Err(ClientError::Api(_))));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let transport = AtlasClient::new(format!("http://{address}"));
        assert!(matches!(
            transport.server_meta().await,
            Err(ClientError::Transport(_))
        ));
    }

    #[tokio::test]
    async fn task_creation_methods_decode_reference_envelopes_and_preserve_task_compatibility() {
        const BOARD_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000010");
        const DOCUMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000020");
        const RESPONSE: &str = r#"{"task":{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","board_id":"00000000-0000-0000-0000-000000000010","column_id":"00000000-0000-0000-0000-000000000004","readable_id":"ATL-1","title":"Source","description":"","labels":[],"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","board_name":"Board","column_name":"Todo"},"references":[{"id":"00000000-0000-0000-0000-000000000006","kind":"relates","target_readable_id":"ATL-2","target_title":"Current target task","target_resolved":true,"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z"},{"id":"00000000-0000-0000-0000-000000000007","kind":"spec","target_document_id":"00000000-0000-0000-0000-000000000020","target_title":"Current target document","target_resolved":true,"created_by":{"type":"api_key","id":"00000000-0000-0000-0000-000000000005","display_name":"Agent","key_type":"agent"},"created_at":"2026-01-01T00:00:00Z"}]}"#;

        let request = CreateTaskRequest {
            column_id: uuid::uuid!("00000000-0000-0000-0000-000000000004"),
            title: "Source".into(),
            description: None,
            properties: None,
            before: None,
            after: None,
            references: vec![
                CreateReferenceRequest {
                    kind: "relates".into(),
                    target_task_readable_id: Some("ATL-2".into()),
                    target_document_id: None,
                },
                CreateReferenceRequest {
                    kind: "spec".into(),
                    target_task_readable_id: None,
                    target_document_id: Some(DOCUMENT_ID),
                },
            ],
        };

        let (base_url, requests) = serve_once_observing("201 Created", RESPONSE);
        let created = AtlasClient::new(base_url)
            .create_task_with_references("ws", BOARD_ID, request.clone())
            .await
            .expect("reference-aware task creation succeeds");

        assert_eq!(created.task.readable_id, "ATL-1");
        assert_eq!(created.references.len(), 2);
        assert_eq!(
            created
                .references
                .first()
                .and_then(|reference| reference.target_title.as_deref()),
            Some("Current target task")
        );
        assert_eq!(
            created
                .references
                .last()
                .and_then(|reference| reference.target_title.as_deref()),
            Some("Current target document")
        );

        let raw = requests.recv().expect("mock server received request");
        assert!(raw.starts_with(
            "POST /api/v2/acta/workspaces/ws/boards/00000000-0000-0000-0000-000000000010/tasks "
        ));
        assert!(raw.contains("\"target_task_readable_id\":\"ATL-2\""));
        assert!(raw.contains("\"target_document_id\":\"00000000-0000-0000-0000-000000000020\""));

        let task = AtlasClient::new(serve_once("201 Created", RESPONSE))
            .create_task("ws", BOARD_ID, request)
            .await
            .expect("task-only compatibility method succeeds");

        assert_eq!(task.readable_id, "ATL-1");
        assert_eq!(task.title, "Source");
    }

    #[tokio::test]
    async fn comment_attachment_lifecycle_methods_use_canonical_task_and_document_routes() {
        const COMMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const ATTACHMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const ATTACHMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000002","comment_id":"00000000-0000-0000-0000-000000000001","file_name":"note.txt","content_type":"text/plain","size_bytes":2,"sha256":"digest","actor":null,"created_at":"2026-01-01T00:00:00Z"}"#;

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        let attachment = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.upload_task_comment_attachment("ws", "ATL-1", COMMENT_ID, "note.txt", "text/plain", b"ok".to_vec()),
        )
        .await;
        assert_eq!(attachment.id, ATTACHMENT_ID);

        let (base_url, requests) = serve_once_observing("200 OK", format!("[{ATTACHMENT}]"));
        let client = AtlasClient::new(base_url);
        let attachments = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.list_task_comment_attachments("ws", "ATL-1", COMMENT_ID),
        )
        .await;
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments.first().map(|attachment| attachment.id),
            Some(ATTACHMENT_ID)
        );

        let (base_url, requests) = serve_once_observing("200 OK", "ok");
        let client = AtlasClient::new(base_url);
        let (data, _) = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002/content ",
            client.download_task_comment_attachment("ws", "ATL-1", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;
        assert_eq!(data, b"ok");

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.delete_task_comment_attachment("ws", "ATL-1", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.upload_document_comment_attachment("ws", "note", COMMENT_ID, "note.txt", "text/plain", b"ok".to_vec()),
        )
        .await;

        let (base_url, requests) = serve_once_observing("200 OK", format!("[{ATTACHMENT}]"));
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            client.list_document_comment_attachments("ws", "note", COMMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("200 OK", "ok");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.download_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            client.delete_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID),
        )
        .await;
    }

    #[tokio::test]
    async fn comment_draft_methods_use_frozen_routes_tokens_and_transports() {
        const DRAFT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const CREATE_TOKEN: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const UPLOAD_TOKEN: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
        const DRAFT: &str =
            r#"{"id":"00000000-0000-0000-0000-000000000001","expires_at":"2026-01-02T00:00:00Z"}"#;
        const ATTACHMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000004","comment_id":"00000000-0000-0000-0000-000000000001","file_name":"note.txt","content_type":"text/plain","size_bytes":2,"sha256":"digest","actor":null,"created_at":"2026-01-01T00:00:00Z","url":"/attachment","markdown":"[note.txt](/attachment)"}"#;

        let (base_url, requests) = serve_once_observing("201 Created", DRAFT);
        let client = AtlasClient::new(base_url);
        let task_draft = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts ",
            &["x-create-token: 00000000-0000-0000-0000-000000000002"],
            client.create_task_comment_draft("ws", "ATL-1", CREATE_TOKEN),
        )
        .await;
        assert_eq!(task_draft.id, DRAFT_ID);

        let (base_url, requests) = serve_once_observing("200 OK", DRAFT);
        let client = AtlasClient::new(base_url);
        let document_draft = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comment-drafts ",
            &["x-create-token: 00000000-0000-0000-0000-000000000002"],
            client.create_document_comment_draft("ws", "note", CREATE_TOKEN),
        )
        .await;
        assert_eq!(
            document_draft.expires_at.to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );

        let (base_url, requests) = serve_once_observing("201 Created", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        let attachment = assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts/00000000-0000-0000-0000-000000000001/attachments ",
            &[
                "x-upload-token: 00000000-0000-0000-0000-000000000003",
                "multipart/form-data; boundary=atlasboundary",
            ],
            client.upload_task_draft_attachment(
                "ws", "ATL-1", DRAFT_ID, UPLOAD_TOKEN, "note.txt", "text/plain", b"ok".to_vec(),
            ),
        )
        .await;
        assert_eq!(
            attachment.markdown.as_deref(),
            Some("[note.txt](/attachment)")
        );

        let (base_url, requests) = serve_once_observing("200 OK", ATTACHMENT);
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request_with_headers(
            requests,
            "POST /api/v2/acta/workspaces/ws/documents/note/comment-drafts/00000000-0000-0000-0000-000000000001/attachments ",
            &[
                "x-upload-token: 00000000-0000-0000-0000-000000000003",
                "x-file-name: note.txt",
                "content-type: text/plain",
            ],
            client.upload_document_draft_attachment(
                "ws", "note", DRAFT_ID, UPLOAD_TOKEN, "note.txt", "text/plain", b"ok".to_vec(),
            ),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/tasks/ATL-1/comment-drafts/00000000-0000-0000-0000-000000000001 ",
            client.cancel_task_comment_draft("ws", "ATL-1", DRAFT_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "DELETE /api/v2/acta/workspaces/ws/documents/note/comment-drafts/00000000-0000-0000-0000-000000000001 ",
            client.cancel_document_comment_draft("ws", "note", DRAFT_ID),
        )
        .await;
    }

    #[tokio::test]
    async fn comment_attachment_lifecycle_methods_preserve_api_decode_and_transport_errors() {
        const COMMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const ATTACHMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");

        let api = AtlasClient::new(serve_once(
            "403 Forbidden",
            r#"{"type":"urn:atlas:error:forbidden","title":"Forbidden","status":403}"#,
        ));
        assert!(matches!(
            api.upload_task_comment_attachment(
                "ws",
                "ATL-1",
                COMMENT_ID,
                "note.txt",
                "text/plain",
                b"ok".to_vec(),
            )
            .await,
            Err(ClientError::Api(_))
        ));

        let decode = AtlasClient::new(serve_once("200 OK", "not attachment metadata"));
        assert!(matches!(
            decode
                .list_document_comment_attachments("ws", "note", COMMENT_ID)
                .await,
            Err(ClientError::Decode {
                context: "list_document_comment_attachments",
                ..
            })
        ));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let transport = AtlasClient::new(format!("http://{address}"));
        assert!(matches!(
            transport
                .delete_document_comment_attachment("ws", "note", COMMENT_ID, ATTACHMENT_ID)
                .await,
            Err(ClientError::Transport(_))
        ));
    }

    #[tokio::test]
    async fn trash_lifecycle_methods_use_typed_admin_routes_and_status_mapping() {
        const TARGET_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
        const OPERATION_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
        const PAGE: &str = r#"{"items":[{"workspace_id":"00000000-0000-0000-0000-000000000003","kind":"document","target_id":"00000000-0000-0000-0000-000000000001","deleted_at":"2026-07-22T00:00:00Z"}],"next_cursor":"next","has_more":true}"#;
        const STATUS: &str = r#"{"operation_id":"00000000-0000-0000-0000-000000000002","kind":"document","target_id":"00000000-0000-0000-0000-000000000001","status":"cleanup_pending","attempts":2}"#;

        let (base_url, requests) = serve_once_observing("200 OK", PAGE);
        let client = AtlasClient::new(base_url);
        let page = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/admin/trash?workspace_id=00000000-0000-0000-0000-000000000003&kind=document&cursor=next&limit=20 ",
            client.list_trash(
                Some(uuid::uuid!("00000000-0000-0000-0000-000000000003")),
                Some(TrashKindDto::Document),
                Some("next"),
                Some(20),
            ),
        )
        .await;
        assert_eq!(
            page.items.first().map(|item| item.target_id),
            Some(TARGET_ID)
        );

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/restore ",
            client.restore_trash(TrashKindDto::Document, TARGET_ID),
        )
        .await;

        let (base_url, requests) = serve_once_observing("202 Accepted", STATUS);
        let client = AtlasClient::new(base_url);
        let response = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/purge ",
            client.purge_trash(TrashKindDto::Document, TARGET_ID, true),
        )
        .await;
        assert!(matches!(
            response,
            PurgeTrashResult::Pending(PurgeStatusDtoResponse {
                status: PurgeStatusDto::CleanupPending,
                ..
            })
        ));

        let (base_url, requests) = serve_once_observing("200 OK", STATUS);
        let client = AtlasClient::new(base_url);
        let status = assert_comment_attachment_request(
            requests,
            "GET /api/v2/acta/admin/trash/purges/00000000-0000-0000-0000-000000000002 ",
            client.get_purge_status(OPERATION_ID),
        )
        .await;
        assert_eq!(status.operation_id, OPERATION_ID);

        let (base_url, requests) = serve_once_observing("204 No Content", "");
        let client = AtlasClient::new(base_url);
        let complete = assert_comment_attachment_request(
            requests,
            "POST /api/v2/acta/admin/trash/purge ",
            client.purge_trash(TrashKindDto::Document, TARGET_ID, true),
        )
        .await;
        assert!(matches!(complete, PurgeTrashResult::Complete));

        let denied = AtlasClient::new(serve_once(
            "403 Forbidden",
            r#"{"type":"urn:atlas:error:forbidden","title":"Forbidden","status":403}"#,
        ));
        assert!(matches!(
            denied.restore_trash(TrashKindDto::Document, TARGET_ID).await,
            Err(ClientError::Api(problem)) if problem.status == 403
        ));
    }

    #[tokio::test]
    async fn bounded_document_methods_serialize_requests_and_decode_continuations() {
        const COMPACT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        const RANGE: &str = r#"{"head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"lines":[{"line_number":3,"text":"third"}],"byte_count":5,"has_more":true,"continuation":"range-next"}"#;
        const SEARCH: &str = r#"{"head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":4,"matches":[{"line_number":5,"preview":"needle"}],"byte_count":6,"has_more":true,"continuation":"search-next"}"#;

        let (base_url, requests) = serve_once_observing("200 OK", COMPACT);
        let compact = AtlasClient::new(base_url)
            .get_document_compact("ws", "note")
            .await
            .expect("compact document request succeeds");
        assert_eq!(compact.title, "Note");
        assert_eq!(compact.head_seq, 4);
        assert!(
            requests
                .recv()
                .expect("mock server received compact request")
                .starts_with("GET /api/v2/acta/workspaces/ws/documents/note/compact ")
        );

        let (base_url, requests) = serve_once_observing("200 OK", RANGE);
        let range = AtlasClient::new(base_url)
            .get_document_content_range(
                "ws",
                "note",
                DocumentContentRangeQuery {
                    start_line: Some(3),
                    end_line: Some(8),
                    line_limit: Some(2),
                    byte_limit: Some(64),
                    continuation: Some("range token".into()),
                },
            )
            .await
            .expect("bounded range request succeeds");
        assert_eq!(range.lines.first().map(|line| line.line_number), Some(3));
        assert_eq!(range.continuation.as_deref(), Some("range-next"));
        let range_request = requests.recv().expect("mock server received range request");
        assert!(range_request.starts_with(
            "GET /api/v2/acta/workspaces/ws/documents/note/content/range?start_line=3&end_line=8&line_limit=2&byte_limit=64&continuation=range%20token "
        ));

        let (base_url, requests) = serve_once_observing("200 OK", SEARCH);
        let search = AtlasClient::new(base_url)
            .search_document_content(
                "ws",
                "note",
                DocumentContentSearchRequest {
                    start_line: Some(2),
                    end_line: Some(9),
                    query: "a+b".into(),
                    mode: Some(atlas_api::dtos::documents::DocumentSearchMode::Pattern),
                    match_limit: Some(3),
                    byte_limit: Some(128),
                    continuation: Some("search-next".into()),
                },
            )
            .await
            .expect("bounded search request succeeds");
        assert_eq!(
            search
                .matches
                .first()
                .map(|matched| matched.preview.as_str()),
            Some("needle")
        );
        assert_eq!(search.continuation.as_deref(), Some("search-next"));
        let search_request = requests
            .recv()
            .expect("mock server received search request");
        assert!(
            search_request
                .starts_with("POST /api/v2/acta/workspaces/ws/documents/note/content/search ")
        );
        assert!(search_request.contains("\"query\":\"a+b\""));
        assert!(search_request.contains("\"mode\":\"pattern\""));
        assert!(search_request.contains("\"continuation\":\"search-next\""));
    }

    #[tokio::test]
    async fn bounded_document_range_omits_absent_query_fields_and_preserves_api_errors() {
        let (base_url, requests) = serve_once_observing(
            "409 Conflict",
            r#"{"type":"urn:atlas:error:conflict","title":"Conflict","status":409,"hint":"restart the range read"}"#,
        );
        let error = AtlasClient::new(base_url)
            .get_document_content_range("ws", "note", DocumentContentRangeQuery::default())
            .await
            .expect_err("stale continuation response is an API error");
        assert!(matches!(error, ClientError::Api(problem) if problem.status == 409));
        assert!(
            requests
                .recv()
                .expect("mock server received range request")
                .starts_with("GET /api/v2/acta/workspaces/ws/documents/note/content/range ")
        );
    }

    #[tokio::test]
    async fn document_line_edit_serializes_tagged_request_and_preserves_conflicts() {
        const DOCUMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let request = atlas_api::dtos::documents::DocumentContentEditRequest {
            base_revision_id: "00000000-0000-0000-0000-000000000003"
                .parse()
                .expect("test revision UUID is valid"),
            edit: atlas_api::dtos::documents::DocumentLineEditRequest::Replace {
                start: 2,
                end: 2,
                content: "updated".into(),
            },
        };

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        let document = AtlasClient::new(base_url)
            .edit_document_content_range("ws", "note", request.clone())
            .await
            .expect("line edit succeeds");
        assert_eq!(document.head_seq, 5);
        assert_eq!(
            document.head_revision_id.to_string(),
            "00000000-0000-0000-0000-000000000003"
        );

        let sent = requests.recv().expect("mock server received edit request");
        assert!(sent.starts_with("PATCH /api/v2/acta/workspaces/ws/documents/note/content/range "));
        assert!(sent.contains("\"base_revision_id\":\"00000000-0000-0000-0000-000000000003\""));
        assert!(sent.contains("\"operation\":\"replace\""));
        assert!(sent.contains("\"start\":2"));
        assert!(sent.contains("\"end\":2"));
        assert!(sent.contains("\"content\":\"updated\""));

        let conflict = AtlasClient::new(serve_once(
            "409 Conflict",
            r#"{"type":"urn:atlas:error:conflict","title":"Conflict","status":409,"current_revision_id":"00000000-0000-0000-0000-000000000004","current_seq":6,"base_to_current_patch":"@@ -1 +1 @@"}"#,
        ))
        .edit_document_content_range("ws", "note", request)
        .await
        .expect_err("stale line edit returns a typed conflict");
        assert!(matches!(conflict, ClientError::Conflict(problem)
            if problem.current_seq == 6
                && problem.base_to_current_patch == "@@ -1 +1 @@"));
    }

    #[tokio::test]
    async fn document_line_edit_serializes_exact_insert_and_delete_request_bodies() {
        const DOCUMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":null,"folder_id":null,"slug":"note","title":"Note","head_revision_id":"00000000-0000-0000-0000-000000000003","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let base_revision_id = "00000000-0000-0000-0000-000000000003"
            .parse()
            .expect("test revision UUID is valid");

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        AtlasClient::new(base_url)
            .edit_document_content_range(
                "ws",
                "note",
                atlas_api::dtos::documents::DocumentContentEditRequest {
                    base_revision_id,
                    edit: atlas_api::dtos::documents::DocumentLineEditRequest::Insert {
                        position: 3,
                        content: "new line".into(),
                    },
                },
            )
            .await
            .expect("insert succeeds");
        let insert: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received insert request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("insert body is JSON");
        assert_eq!(
            insert,
            serde_json::json!({
                "base_revision_id": "00000000-0000-0000-0000-000000000003",
                "operation": "insert",
                "position": 3,
                "content": "new line",
            })
        );

        let (base_url, requests) = serve_once_observing("200 OK", DOCUMENT);
        AtlasClient::new(base_url)
            .edit_document_content_range(
                "ws",
                "note",
                atlas_api::dtos::documents::DocumentContentEditRequest {
                    base_revision_id,
                    edit: atlas_api::dtos::documents::DocumentLineEditRequest::Delete {
                        start: 2,
                        end: 4,
                    },
                },
            )
            .await
            .expect("delete succeeds");
        let delete: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received delete request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("delete body is JSON");
        assert_eq!(
            delete,
            serde_json::json!({
                "base_revision_id": "00000000-0000-0000-0000-000000000003",
                "operation": "delete",
                "start": 2,
                "end": 4,
            })
        );
    }

    #[tokio::test]
    async fn document_line_edit_preserves_validation_hints() {
        let request = atlas_api::dtos::documents::DocumentContentEditRequest {
            base_revision_id: "00000000-0000-0000-0000-000000000003"
                .parse()
                .expect("test revision UUID is valid"),
            edit: atlas_api::dtos::documents::DocumentLineEditRequest::Insert {
                position: 1,
                content: "new first line".into(),
            },
        };
        let error = AtlasClient::new(serve_once(
            "422 Unprocessable Content",
            r#"{"type":"urn:atlas:error:validation","title":"Validation failed","status":422,"hint":"position must be within the document line range"}"#,
        ))
        .edit_document_content_range("ws", "note", request)
        .await
        .expect_err("invalid line edit returns the server validation problem");

        assert!(matches!(
            error,
            ClientError::Api(problem)
                if problem.status == 422
                    && problem.hint.as_deref()
                        == Some("position must be within the document line range")
        ));
    }

    #[tokio::test]
    async fn document_move_batch_serializes_ordered_moves_and_decodes_mixed_outcomes() {
        const RESPONSE: &str = r#"[{"outcome":"success","index":0,"document":{"id":"00000000-0000-0000-0000-000000000001","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","folder_id":"00000000-0000-0000-0000-000000000004","slug":"first","title":"Current first title","head_revision_id":"00000000-0000-0000-0000-000000000005","head_seq":5,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}},{"outcome":"problem","index":1,"status":404,"type":"urn:atlas:error:not-found","title":"Not found","hint":"document is unavailable"},{"outcome":"success","index":2,"document":{"id":"00000000-0000-0000-0000-000000000006","workspace_id":"00000000-0000-0000-0000-000000000002","project_id":"00000000-0000-0000-0000-000000000003","folder_id":null,"slug":"third","title":"Current third title","head_revision_id":"00000000-0000-0000-0000-000000000007","head_seq":7,"frontmatter":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}}]"#;

        let destination_folder = uuid::uuid!("00000000-0000-0000-0000-000000000004");
        let request = DocumentMoveBatchRequest {
            moves: vec![
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "first".into(),
                    folder_id: Some(destination_folder),
                },
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "missing".into(),
                    folder_id: Some(destination_folder),
                },
                atlas_api::dtos::documents::DocumentMoveBatchItemRequest {
                    source_document: "third".into(),
                    folder_id: None,
                },
            ],
        };
        let (base_url, requests) = serve_once_observing("200 OK", RESPONSE);

        let outcomes = AtlasClient::new(base_url)
            .move_documents_batch("ws", request)
            .await
            .expect("batch move succeeds with per-item outcomes");

        assert!(matches!(
            outcomes.as_slice(),
            [
                DocumentMoveBatchResultDto::Success { index: 0, document },
                DocumentMoveBatchResultDto::Problem {
                    index: 1,
                    status: 404,
                    r#type,
                    title,
                    hint: Some(hint),
                },
                DocumentMoveBatchResultDto::Success { index: 2, .. },
            ] if document.title == "Current first title"
                && r#type == "urn:atlas:error:not-found"
                && title == "Not found"
                && hint == "document is unavailable"
        ));

        let sent: serde_json::Value = serde_json::from_str(
            requests
                .recv()
                .expect("mock server received batch move request")
                .split("\r\n\r\n")
                .nth(1)
                .expect("request includes a JSON body"),
        )
        .expect("batch move body is JSON");
        assert_eq!(
            sent,
            serde_json::json!({
                "moves": [
                    {"source_document": "first", "folder_id": destination_folder},
                    {"source_document": "missing", "folder_id": destination_folder},
                    {"source_document": "third", "folder_id": null},
                ]
            })
        );
    }
}
