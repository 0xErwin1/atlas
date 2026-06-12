#![allow(dead_code)]

pub(crate) mod route_matrix;

use atlas_client::AtlasClient;
use atlas_domain::{Actor, WorkspaceCtx, ids::WorkspaceId};
use atlas_server::{
    persistence::repos::{
        MembershipRepo, NewUser, NewWorkspace, PgApiKeyRepo, PgBoardRepo, PgFolderRepo,
        PgMembershipRepo, PgProjectRepo, PgPropertyDefinitionRepo, PgSessionRepo, PgTaskRepo,
        PgUserRepo, PgWorkspaceRepo, User, UserRepo, Workspace, WorkspaceRepo,
    },
    state::AppState,
};
use migration::Migrator;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr};
use sea_orm_migration::prelude::MigratorTrait;
use tokio::task::AbortHandle;
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

    pub(crate) fn user_repo(&self) -> PgUserRepo {
        PgUserRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn workspace_repo(&self) -> PgWorkspaceRepo {
        PgWorkspaceRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn session_repo(&self) -> PgSessionRepo {
        PgSessionRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn api_key_repo(&self) -> PgApiKeyRepo {
        PgApiKeyRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn membership_repo(&self) -> PgMembershipRepo {
        PgMembershipRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn project_repo(&self) -> PgProjectRepo {
        PgProjectRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn folder_repo(&self) -> PgFolderRepo {
        PgFolderRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn property_definition_repo(&self) -> PgPropertyDefinitionRepo {
        PgPropertyDefinitionRepo {
            conn: self.conn.clone(),
        }
    }

    pub(crate) fn board_repo(&self) -> PgBoardRepo {
        PgBoardRepo::new(self.conn.clone())
    }

    pub(crate) fn task_repo(&self) -> PgTaskRepo {
        PgTaskRepo::new(self.conn.clone())
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

pub(crate) async fn seed_workspace(db: &TestDb, username: &str) -> (Workspace, User) {
    use atlas_domain::entities::identity::MemberRole;

    let user_repo = db.user_repo();
    let ws_repo = db.workspace_repo();
    let membership_repo = db.membership_repo();

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
        })
        .await
        .expect("seed user");

    let slug = format!("ws-{username}");
    let ws_id = WorkspaceId::new();
    let ws = ws_repo
        .create(NewWorkspace {
            id: ws_id,
            name: format!("Workspace {username}"),
            slug: slug.clone(),
        })
        .await
        .expect("seed workspace");

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    membership_repo
        .add(&ctx, user.id, MemberRole::Owner)
        .await
        .expect("seed membership");

    (ws, user)
}

pub(crate) fn ctx(ws: &Workspace, user: &User) -> WorkspaceCtx {
    WorkspaceCtx::new(ws.id, Actor::User(user.id))
}

/// A live test server bound to a random port on 127.0.0.1.
///
/// The server task is aborted when `TestServer` is dropped.
pub(crate) struct TestServer {
    base_url: String,
    _abort: AbortHandle,
}

impl TestServer {
    /// Spawns the application on a random available port using the test database.
    pub(crate) async fn spawn(db: &TestDb) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test port");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{addr}");

        use std::net::SocketAddr;
        let state = AppState::for_test(db.conn().clone());
        let app = atlas_server::app(state).into_make_service_with_connect_info::<SocketAddr>();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        Self {
            base_url,
            _abort: handle.abort_handle(),
        }
    }

    /// Returns an unauthenticated `AtlasClient` pointed at this server.
    pub(crate) fn client(&self) -> AtlasClient {
        AtlasClient::new(self.base_url.clone())
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self._abort.abort();
    }
}

/// Creates a user with a real password hash, starts the server login flow, and
/// returns an authenticated client plus the created user record.
pub(crate) async fn login_user(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (AtlasClient, User) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let password_plaintext = "TestPassword1!";
    let password_hash = password::hash(password_plaintext.to_string())
        .await
        .expect("hash password");

    let user_repo = db.user_repo();
    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            password_hash,
            is_root: false,
        })
        .await
        .expect("create user");

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: password_plaintext.to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

/// Seeds a workspace+membership for `username`, then logs in via the test server.
///
/// Returns (authenticated client, workspace, user). Unlike `login_user`, this
/// helper guarantees the user is a workspace owner, making it suitable for tests
/// that exercise workspace-scoped extractors.
pub(crate) async fn login_user_with_workspace(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (AtlasClient, Workspace, User) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let password_plaintext = "TestPassword1!";
    let password_hash = password::hash(password_plaintext.to_string())
        .await
        .expect("hash password");

    let user_repo = db.user_repo();
    let ws_repo = db.workspace_repo();
    let membership_repo = db.membership_repo();

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            password_hash,
            is_root: false,
        })
        .await
        .expect("create user");

    let ws_id = WorkspaceId::new();
    let ws_slug = format!("ws-{username}");
    let ws = ws_repo
        .create(NewWorkspace {
            id: ws_id,
            name: format!("Workspace {username}"),
            slug: ws_slug,
        })
        .await
        .expect("create workspace");

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    membership_repo
        .add(
            &ctx,
            user.id,
            atlas_domain::entities::identity::MemberRole::Owner,
        )
        .await
        .expect("seed membership");

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: password_plaintext.to_string(),
        })
        .await
        .expect("login");

    (client, ws, user)
}

/// Expires all sessions in the test database immediately.
pub(crate) async fn expire_all_sessions(db: &TestDb) {
    db.conn()
        .execute_unprepared("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .await
        .expect("expire sessions");
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
