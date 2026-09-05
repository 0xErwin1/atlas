use atlas_api::dtos::{DoctorReportDto, ServerMetaDto, UiStateDto, UpdateUiStateRequest};

use crate::{AtlasClient, ClientError, Component, Req};

/// The platform-owned methods on [`AtlasClient`]: per-user UI state, server
/// metadata, and the cross-component doctor report — every method whose
/// route is mounted at `/api/v2/platform`. Borrows the root client rather
/// than owning any state of its own, so authentication and CSRF
/// configuration stay single-point on [`AtlasClient`] (INV-SINGLE-AUTH-CONFIG).
pub struct Platform<'a>(pub(crate) &'a AtlasClient);

impl Platform<'_> {
    fn get(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.get(component, relative)
    }

    fn post(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.post(component, relative)
    }

    fn put(&self, component: Component, relative: &str) -> Req<'_> {
        self.0.put(component, relative)
    }

    async fn decode_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        context: &'static str,
    ) -> Result<T, ClientError> {
        self.0.decode_response(response, context).await
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

    /// `POST /api/v2/platform/doctor`
    ///
    /// Runs every present component's doctor check. Platform admin or root
    /// only. Always 200 on success, whether or not findings are present
    /// (SHELL-OPS-4) — the CLI's exit code, not the HTTP status, signals a
    /// `Critical` finding.
    pub async fn doctor(&self) -> Result<DoctorReportDto, ClientError> {
        let response = self
            .post(Component::Platform, "/doctor")
            .header("x-atlas-csrf", "1")
            .send()
            .await?;
        self.decode_response(response, "doctor").await
    }
}
