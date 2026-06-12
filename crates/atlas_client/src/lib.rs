#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use atlas_api::{
    dtos::{HealthResponse, LoginRequest, LoginResponse, MeResponse},
    problem::ProblemDetails,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("API error {}: {}", .0.status, .0.title)]
    Api(ProblemDetails),
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

    /// `GET /v1/workspaces/{ws}/probe`
    pub async fn get_probe(&self, ws: &str) -> Result<(), ClientError> {
        let response = self
            .get(&format!("/v1/workspaces/{ws}/probe"))
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
}
