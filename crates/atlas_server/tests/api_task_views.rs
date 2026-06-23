#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateUserApiKeyRequest, InitialGrantRequest,
    task_views::{CreateTaskViewRequest, TaskViewFiltersDto, UpdateTaskViewRequest},
};
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// TV01: create with non-default filters → 201, appears in list, filters round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_returns_201_and_appears_in_list_with_filters_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-create-1").await;

    let filters = TaskViewFiltersDto {
        sort: Some("updated_at_desc".to_string()),
        priorities: vec!["high".to_string(), "urgent".to_string()],
        ..Default::default()
    };

    let view = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "High Priority Tasks".to_string(),
                filters: filters.clone(),
            },
        )
        .await
        .expect("create task view");

    assert_eq!(view.name, "High Priority Tasks");
    assert_eq!(view.workspace_id, ws.id.0);
    assert_eq!(view.filters.sort.as_deref(), Some("updated_at_desc"));
    assert_eq!(view.filters.priorities, vec!["high", "urgent"]);

    let listed = client
        .list_task_views(&ws.slug)
        .await
        .expect("list task views");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, view.id);
    assert_eq!(listed[0].name, "High Priority Tasks");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV02: blank name → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_rejects_blank_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-blank-name-1").await;

    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "   ".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "blank name must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV03: name > 200 chars → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_rejects_name_over_200_chars() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-long-name-1").await;

    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "a".repeat(201),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "name > 200 chars must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV04: invalid filters → 422 (unknown sort key, oversized collection, invalid priority)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_rejects_invalid_filters() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-invalid-filters-1").await;

    // Unknown sort key
    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Bad Sort".to_string(),
                filters: TaskViewFiltersDto {
                    sort: Some("not_a_valid_sort".to_string()),
                    ..Default::default()
                },
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "unknown sort key must be rejected as 422, got {result:?}"
    );

    // Oversized column_ids collection (> 50)
    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Too Many Columns".to_string(),
                filters: TaskViewFiltersDto {
                    column_ids: (0..51).map(|_| uuid::Uuid::now_v7()).collect(),
                    ..Default::default()
                },
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "column_ids > 50 entries must be rejected as 422, got {result:?}"
    );

    // Invalid priority string
    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Bad Priority".to_string(),
                filters: TaskViewFiltersDto {
                    priorities: vec!["not_a_priority".to_string()],
                    ..Default::default()
                },
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid priority string must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV05: empty filters {} → 201
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_allows_empty_filters() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-empty-filters-1").await;

    let view = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "All Tasks".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("empty filters must be accepted");

    assert_eq!(view.name, "All Tasks");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV06: duplicate name same owner → 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_rejects_duplicate_name_for_same_owner() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-dup-name-1").await;

    client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "My View".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("first create");

    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "My View".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate name must return 409, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV07: same name, different owners (user vs api_key) → both 201
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_allows_same_name_for_different_owners() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (user_client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-cross-owner-1").await;

    user_client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Shared Name".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("user creates task view");

    let key_created = user_client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "test-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = key_client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Shared Name".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "api_key owner must be allowed same name as user owner, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV08: list is owner-scoped, alphabetical, excludes soft-deleted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_task_views_is_owner_scoped_sorted_and_excludes_deleted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-list-scoped-1").await;

    for name in ["Gamma", "alpha", "Beta"] {
        client
            .create_task_view(
                &ws.slug,
                CreateTaskViewRequest {
                    name: name.to_string(),
                    filters: TaskViewFiltersDto::default(),
                },
            )
            .await
            .expect("create");
    }

    // Create an api_key owner's view (should not appear in user's list)
    let key_created = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "other-owner-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    key_client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "ZOther".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("key owner creates");

    // Create a view to be deleted
    let to_delete = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "ToDelete".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create to-delete");

    client
        .delete_task_view(&ws.slug, to_delete.id)
        .await
        .expect("delete");

    let listed = client.list_task_views(&ws.slug).await.expect("list");

    let names: Vec<String> = listed.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names, vec!["alpha", "Beta", "Gamma"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV09: GET /{id} → 200, filters present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_task_view_returns_200_with_filters() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-get-by-id-1").await;

    let created = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "My Filters".to_string(),
                filters: TaskViewFiltersDto {
                    sort: Some("created_at_desc".to_string()),
                    priorities: vec!["low".to_string()],
                    ..Default::default()
                },
            },
        )
        .await
        .expect("create");

    let fetched = client
        .get_task_view(&ws.slug, created.id)
        .await
        .expect("get by id");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.filters.sort.as_deref(), Some("created_at_desc"));
    assert_eq!(fetched.filters.priorities, vec!["low"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV10: GET /{id} non-owned → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_task_view_returns_404_for_non_owned_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-get-nonowned-1").await;

    let view = client_a
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Mine".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    let key_created = client_a
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "intruder-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let client_b = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = client_b.get_task_view(&ws.slug, view.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "get of non-owned id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV11: PATCH → 200, both name and filters changed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_view_returns_200_with_new_name_and_filters() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-update-1").await;

    let created = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Old Name".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    let updated = client
        .update_task_view(
            &ws.slug,
            created.id,
            UpdateTaskViewRequest {
                name: "New Name".to_string(),
                filters: TaskViewFiltersDto {
                    sort: Some("title_asc".to_string()),
                    priorities: vec!["medium".to_string()],
                    ..Default::default()
                },
            },
        )
        .await
        .expect("update");

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.filters.sort.as_deref(), Some("title_asc"));
    assert_eq!(updated.filters.priorities, vec!["medium"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV12: PATCH → duplicate name → 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_view_rejects_duplicate_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-update-dup-1").await;

    client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Alpha".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create alpha");

    let beta = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Beta".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create beta");

    let result = client
        .update_task_view(
            &ws.slug,
            beta.id,
            UpdateTaskViewRequest {
                name: "Alpha".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "update into duplicate name must return 409, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV13: PATCH missing id → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_view_returns_404_for_missing_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-update-missing-1").await;

    let missing_id = uuid::Uuid::now_v7();
    let result = client
        .update_task_view(
            &ws.slug,
            missing_id,
            UpdateTaskViewRequest {
                name: "Ghost".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "update of missing id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV14: PATCH non-owned id → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_view_returns_404_for_non_owned_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-update-nonowned-1").await;

    let view = client_a
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Mine".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    let key_created = client_a
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "intruder-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let client_b = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = client_b
        .update_task_view(
            &ws.slug,
            view.id,
            UpdateTaskViewRequest {
                name: "Stolen".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "update of non-owned id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV15: DELETE → 204, absent in list, name re-creatable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_task_view_returns_204_and_frees_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-delete-1").await;

    let view = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "To Delete".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    client
        .delete_task_view(&ws.slug, view.id)
        .await
        .expect("delete must return 204");

    let listed = client
        .list_task_views(&ws.slug)
        .await
        .expect("list after delete");

    assert!(listed.is_empty(), "deleted row must not appear in list");

    client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "To Delete".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("re-creating with same name must succeed after soft-delete");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV16: DELETE non-owned → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_task_view_returns_404_for_non_owned_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-delete-nonowned-1").await;

    let view = client_a
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Mine".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    let key_created = client_a
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "intruder-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let client_b = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = client_b.delete_task_view(&ws.slug, view.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "delete of non-owned id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV17: DELETE missing id → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_task_view_returns_404_for_missing_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-delete-missing-1").await;

    let missing_id = uuid::Uuid::now_v7();
    let result = client.delete_task_view(&ws.slug, missing_id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "delete of missing id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV18: double delete → second returns 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_task_view_returns_404_on_second_delete() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-double-delete-1").await;

    let view = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "ToDelete".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create");

    client
        .delete_task_view(&ws.slug, view.id)
        .await
        .expect("first delete must return 204");

    let result = client.delete_task_view(&ws.slug, view.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "second delete of soft-deleted id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV19: unauthenticated → 401 on all endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_views_endpoints_reject_unauthenticated_requests() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner_client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tv-unauth-1").await;

    let anon = server.client();

    let result = anon.list_task_views(&ws.slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated list must return 401, got {result:?}"
    );

    let result = anon
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "x".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated create must return 401, got {result:?}"
    );

    let view = owner_client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Auth Test".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await
        .expect("create for auth test");

    let result = anon.get_task_view(&ws.slug, view.id).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated get must return 401, got {result:?}"
    );

    let result = anon
        .update_task_view(
            &ws.slug,
            view.id,
            UpdateTaskViewRequest {
                name: "X".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated update must return 401, got {result:?}"
    );

    let result = anon.delete_task_view(&ws.slug, view.id).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated delete must return 401, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV20: non-member → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_views_return_404_for_non_member() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_, ws, _) = support::login_user_with_workspace(&server, &db, "tv-nonmember-ws-1").await;
    let (outsider, _, _) =
        support::login_user_with_workspace(&server, &db, "tv-nonmember-user-1").await;

    let result = outsider.list_task_views(&ws.slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member list must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TV21: per-owner cap of 50 → 51st returns 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_view_rejects_over_cap() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tv-cap-1").await;

    for i in 0..50 {
        client
            .create_task_view(
                &ws.slug,
                CreateTaskViewRequest {
                    name: format!("View {i}"),
                    filters: TaskViewFiltersDto::default(),
                },
            )
            .await
            .expect("create within cap");
    }

    let result = client
        .create_task_view(
            &ws.slug,
            CreateTaskViewRequest {
                name: "Over Cap".to_string(),
                filters: TaskViewFiltersDto::default(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "51st task view must return 422, got {result:?}"
    );

    db.teardown().await;
}
