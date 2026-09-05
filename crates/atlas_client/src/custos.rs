use atlas_api::{
    dtos::{
        ActivationLinkResponse, ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, ApiKeyScope,
        ChangePasswordRequest, CreateGrantRequest, CreateUserApiKeyRequest, CreateUserRequest,
        CreateUserResponse, GrantDto, MeResponse, ResetPasswordRequest, UpdateMeRequest, UserDto,
        UserMembershipDto,
        groups::{AddGroupMemberRequest, CreateGroupRequest, GroupDto, GroupMemberDto},
    },
    pagination::Page,
    problem::ProblemDetails,
};

use crate::{AtlasClient, ClientError, Component, Req, build_audit_path, build_paginated_path};

/// The custos-owned methods on [`AtlasClient`]: authentication, users, API
/// keys, grants, and groups — every method whose route is mounted at
/// `/api/v2/custos`. Borrows the root client rather than owning any state of
/// its own, so authentication and CSRF configuration stay single-point on
/// [`AtlasClient`] (INV-SINGLE-AUTH-CONFIG).
pub struct Custos<'a>(pub(crate) &'a AtlasClient);

impl Custos<'_> {
    fn get(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.get(component, relative)
    }

    fn post(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.post(component, relative)
    }

    fn patch(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.patch(component, relative)
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
