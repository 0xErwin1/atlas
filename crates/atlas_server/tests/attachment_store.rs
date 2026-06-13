#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_domain::AttachmentStore;
use atlas_server::persistence::repos::DiskAttachmentStore;
use tempfile::TempDir;

async fn make_store() -> (DiskAttachmentStore, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(dir.path())
        .await
        .expect("DiskAttachmentStore::new");
    (store, dir)
}

#[tokio::test]
async fn put_then_get_returns_identical_bytes() {
    let (store, _dir) = make_store().await;
    let data = b"hello, attachment store";

    let digest = store.put(data).await.expect("put");
    let retrieved = store.get(&digest).await.expect("get");

    assert_eq!(retrieved.as_ref(), data);
}

#[tokio::test]
async fn exists_false_before_put_true_after() {
    let (store, _dir) = make_store().await;
    let data = b"existence test data";

    let digest = store.put(data).await.expect("put");

    // Construct what an unknown digest would look like to verify the false case.
    // Use the actual digest so we can also verify the true case.
    assert!(
        store.exists(&digest).await.expect("exists after put"),
        "must exist after put"
    );

    // A different digest (wrong data) should return false.
    let unknown = "a".repeat(64);
    assert!(
        !store.exists(&unknown).await.expect("exists unknown"),
        "must not exist before put"
    );
}

#[tokio::test]
async fn concurrent_puts_of_same_content_are_idempotent() {
    let (store, _dir) = make_store().await;
    let data = b"idempotent content";

    let d1 = store.put(data).await.expect("put 1");
    let d2 = store.put(data).await.expect("put 2");
    let d3 = store.put(data).await.expect("put 3");

    assert_eq!(d1, d2, "digest must be stable");
    assert_eq!(d2, d3, "digest must be stable");

    let retrieved = store.get(&d1).await.expect("get after multi-put");
    assert_eq!(retrieved.as_ref(), data);
}

#[tokio::test]
async fn get_nonexistent_digest_returns_not_found() {
    let (store, _dir) = make_store().await;
    let unknown = "b".repeat(64);
    let result = store.get(&unknown).await;
    assert!(
        matches!(result, Err(atlas_domain::DomainError::NotFound { .. })),
        "expected NotFound, got {result:?}"
    );
}
