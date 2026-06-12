use std::fmt;

pub struct ServerConfig {
    pub database_url: String,
    pub root_password: Option<String>,
    pub anchor_interval: u32,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;

        let root_password = std::env::var("ATLAS_ROOT_PASSWORD").ok();

        let anchor_interval = std::env::var("ATLAS_ANCHOR_INTERVAL")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(50);

        if anchor_interval < 2 {
            return Err(format!(
                "ATLAS_ANCHOR_INTERVAL must be >= 2, got {anchor_interval}"
            ));
        }

        Ok(Self {
            database_url,
            root_password,
            anchor_interval,
        })
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("database_url", &self.database_url)
            .field("root_password", &"[REDACTED]")
            .field("anchor_interval", &self.anchor_interval)
            .finish()
    }
}
