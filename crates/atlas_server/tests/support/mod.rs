#![allow(dead_code)]

use migration::Migrator;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

pub(crate) struct TestDb {
    conn: DatabaseConnection,
    db_name: String,
    admin_url: String,
}

impl TestDb {
    pub(crate) async fn create() -> Result<Self, DbErr> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://atlas:atlas@localhost:5432/atlas_dev".to_string());

        let admin_url = admin_url_from(&database_url);
        let db_name = format!("atlas_test_{}", Uuid::now_v7().as_simple());

        let admin = Database::connect(&admin_url).await?;
        admin
            .execute_unprepared(&format!("CREATE DATABASE \"{db_name}\""))
            .await?;
        drop(admin);

        let test_url = replace_db_name(&database_url, &db_name);
        let opts = ConnectOptions::new(test_url);
        let conn = Database::connect(opts).await?;

        Migrator::up(&conn, None).await?;

        Ok(Self {
            conn,
            db_name,
            admin_url,
        })
    }

    pub(crate) fn conn(&self) -> &DatabaseConnection {
        &self.conn
    }

    pub(crate) async fn teardown(self) {
        drop(self.conn);

        if let Ok(admin) = Database::connect(&self.admin_url).await {
            let _ = admin
                .execute_unprepared(&format!(
                    "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                    self.db_name
                ))
                .await;
        }
    }
}

fn admin_url_from(url: &str) -> String {
    replace_db_name(url, "postgres")
}

fn replace_db_name(url: &str, new_db: &str) -> String {
    if let Some(slash_pos) = url.rfind('/') {
        let base = &url[..=slash_pos];
        let rest = &url[slash_pos + 1..];
        let db_only = rest.split('?').next().unwrap_or(rest);
        let query = if rest.contains('?') {
            &rest[db_only.len()..]
        } else {
            ""
        };
        format!("{base}{new_db}{query}")
    } else {
        url.to_owned()
    }
}
