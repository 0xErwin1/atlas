#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// execute_folders, execute_documents_create_path, and the private helpers they
// depend on are wired into the execute orchestrator in B1b. The dead_code
// allow covers the transition period.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use atlas_api::dtos::{documents::CreateDocumentRequest, folders::CreateFolderRequest};
use atlas_client::AtlasClient;
use uuid::Uuid;

use crate::error::CliError;

use super::manifest::{Manifest, ManifestDocEntry, content_hash};
use super::plan::{DocAction, DocumentOp, FolderOp};

type FolderKey = (String, Option<Uuid>);

/// Returns the id of an already-existing folder whose `(name, parent_folder_id)`
/// matches the supplied key, or `None` when no match is found.
///
/// Factored as a pure function so the "reuse existing vs create" decision
/// path can be tested without a live server.
pub(crate) fn find_existing_folder(
    map: &HashMap<FolderKey, Uuid>,
    name: &str,
    parent_id: Option<Uuid>,
) -> Option<Uuid> {
    map.get(&(name.to_string(), parent_id)).copied()
}

/// Paginates `list_folders` into a lookup map keyed by `(name, parent_folder_id)`.
async fn load_existing_folders(
    client: &AtlasClient,
    ws: &str,
    project: &str,
) -> Result<HashMap<FolderKey, Uuid>, CliError> {
    let mut map: HashMap<FolderKey, Uuid> = HashMap::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_folders(ws, project, cursor.as_deref(), Some(100))
            .await?;

        for folder in page.items {
            map.insert((folder.name, folder.parent_folder_id), folder.id);
        }

        if !page.has_more {
            break;
        }

        cursor = page.next_cursor;
    }

    Ok(map)
}

/// Creates Atlas folders for all ops in `folder_ops`, reusing any that already
/// exist by `(name, parent_folder_id)` match.
///
/// `folder_ops` must be in topological order (parent before child), which
/// `build_plan` already guarantees. Each resolved folder id is persisted in
/// `manifest.folders` and the manifest is atomically saved after each
/// successful operation.
pub(crate) async fn execute_folders(
    client: &AtlasClient,
    ws: &str,
    project: &str,
    folder_ops: &[FolderOp],
    manifest: &mut Manifest,
    manifest_path: &Path,
) -> Result<(), CliError> {
    let existing = load_existing_folders(client, ws, project).await?;

    for op in folder_ops {
        let parent_id = resolve_parent_folder_id(&op.parent_rel, manifest)?;

        let folder_id = match find_existing_folder(&existing, &op.name, parent_id) {
            Some(id) => id,
            None => {
                let dto = client
                    .create_folder(
                        ws,
                        project,
                        CreateFolderRequest {
                            name: op.name.clone(),
                            parent_folder_id: parent_id,
                        },
                    )
                    .await?;
                dto.id
            }
        };

        let rel_key = op.rel_path.to_string_lossy().into_owned();
        manifest.folders.insert(rel_key, folder_id.to_string());
        manifest.save(manifest_path)?;
    }

    Ok(())
}

/// Executes only `DocAction::Create` ops; Update ops are skipped until B1b.
///
/// For each Create op: resolves the owning folder from the manifest, calls
/// `create_document`, and persists the server-returned slug and id alongside
/// the content hash. The manifest is atomically saved after each successful
/// create so a partial run can be resumed.
pub(crate) async fn execute_documents_create_path(
    client: &AtlasClient,
    ws: &str,
    project: &str,
    doc_ops: &[DocumentOp],
    manifest: &mut Manifest,
    manifest_path: &Path,
) -> Result<(), CliError> {
    for op in doc_ops {
        if !matches!(op.action, DocAction::Create) {
            continue;
        }

        let folder_id = resolve_doc_folder_id(&op.folder_rel, manifest)?;

        let dto = client
            .create_document(
                ws,
                project,
                CreateDocumentRequest {
                    title: op.title.clone(),
                    folder_id,
                    content: Some(op.content.clone()),
                },
            )
            .await?;

        // The server-returned slug (not the predicted one) is persisted so
        // slug collisions resolved server-side are recorded correctly.
        let slug = dto.slug.ok_or_else(|| {
            CliError::Validation(format!(
                "server returned no slug for document '{}'; cannot persist manifest entry",
                op.title
            ))
        })?;

        let hash = content_hash(&op.content);
        let rel_key = op.rel_path.to_string_lossy().into_owned();

        manifest.documents.insert(
            rel_key,
            ManifestDocEntry {
                slug,
                id: dto.id.to_string(),
                content_hash: hash,
            },
        );
        manifest.save(manifest_path)?;
    }

    Ok(())
}

fn resolve_parent_folder_id(
    parent_rel: &Option<std::path::PathBuf>,
    manifest: &Manifest,
) -> Result<Option<Uuid>, CliError> {
    match parent_rel {
        None => Ok(None),
        Some(parent_path) => {
            let key = parent_path.to_string_lossy().into_owned();
            let id_str = manifest.folders.get(&key).ok_or_else(|| {
                CliError::Validation(format!(
                    "parent folder '{key}' not yet in manifest; folders must be created in \
                     topological order"
                ))
            })?;
            let id = id_str.parse::<Uuid>().map_err(|e| {
                CliError::Validation(format!("invalid folder UUID in manifest for '{key}': {e}"))
            })?;
            Ok(Some(id))
        }
    }
}

fn resolve_doc_folder_id(
    folder_rel: &Option<std::path::PathBuf>,
    manifest: &Manifest,
) -> Result<Option<Uuid>, CliError> {
    match folder_rel {
        None => Ok(None),
        Some(folder_path) => {
            let key = folder_path.to_string_lossy().into_owned();
            let id_str = manifest.folders.get(&key).ok_or_else(|| {
                CliError::Validation(format!(
                    "folder '{key}' not found in manifest; run folder creation before documents"
                ))
            })?;
            let id = id_str.parse::<Uuid>().map_err(|e| {
                CliError::Validation(format!("invalid folder UUID in manifest for '{key}': {e}"))
            })?;
            Ok(Some(id))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn make_map(entries: &[(&str, Option<u128>, u128)]) -> HashMap<FolderKey, Uuid> {
        entries
            .iter()
            .map(|(name, parent, id)| ((name.to_string(), parent.map(uuid)), uuid(*id)))
            .collect()
    }

    // -- find_existing_folder --------------------------------------------------

    #[test]
    fn find_existing_folder_returns_uuid_on_exact_match() {
        let map = make_map(&[("epics", None, 1)]);
        assert_eq!(find_existing_folder(&map, "epics", None), Some(uuid(1)));
    }

    #[test]
    fn find_existing_folder_returns_none_on_name_mismatch() {
        let map = make_map(&[("epics", None, 1)]);
        assert_eq!(find_existing_folder(&map, "tasks", None), None);
    }

    #[test]
    fn find_existing_folder_distinguishes_none_from_some_parent() {
        let parent_id = uuid(99);
        let map = make_map(&[("child", Some(99), 2)]);
        assert_eq!(find_existing_folder(&map, "child", None), None);
        assert_eq!(
            find_existing_folder(&map, "child", Some(parent_id)),
            Some(uuid(2))
        );
    }

    #[test]
    fn find_existing_folder_distinguishes_different_parent_ids() {
        let map = make_map(&[("child", Some(10), 100), ("child", Some(20), 200)]);
        assert_eq!(
            find_existing_folder(&map, "child", Some(uuid(10))),
            Some(uuid(100))
        );
        assert_eq!(
            find_existing_folder(&map, "child", Some(uuid(20))),
            Some(uuid(200))
        );
    }

    #[test]
    fn find_existing_folder_returns_none_on_empty_map() {
        let map: HashMap<FolderKey, Uuid> = HashMap::new();
        assert_eq!(find_existing_folder(&map, "any", None), None);
    }

    // -- resolve_parent_folder_id ----------------------------------------------

    #[test]
    fn resolve_parent_none_returns_none() {
        let m = Manifest::empty();
        assert_eq!(resolve_parent_folder_id(&None, &m).unwrap(), None);
    }

    #[test]
    fn resolve_parent_missing_from_manifest_returns_validation_error() {
        let m = Manifest::empty();
        let path = Some(std::path::PathBuf::from("sub"));
        let err = resolve_parent_folder_id(&path, &m).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn resolve_parent_present_in_manifest_returns_uuid() {
        let mut m = Manifest::empty();
        let id = Uuid::new_v4();
        m.folders.insert("sub".into(), id.to_string());
        let path = Some(std::path::PathBuf::from("sub"));
        assert_eq!(resolve_parent_folder_id(&path, &m).unwrap(), Some(id));
    }

    // -- resolve_doc_folder_id -------------------------------------------------

    #[test]
    fn resolve_doc_folder_none_returns_none() {
        let m = Manifest::empty();
        assert_eq!(resolve_doc_folder_id(&None, &m).unwrap(), None);
    }

    #[test]
    fn resolve_doc_folder_missing_returns_validation_error() {
        let m = Manifest::empty();
        let path = Some(std::path::PathBuf::from("docs"));
        let err = resolve_doc_folder_id(&path, &m).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn resolve_doc_folder_present_returns_uuid() {
        let mut m = Manifest::empty();
        let id = Uuid::new_v4();
        m.folders.insert("docs".into(), id.to_string());
        let path = Some(std::path::PathBuf::from("docs"));
        assert_eq!(resolve_doc_folder_id(&path, &m).unwrap(), Some(id));
    }
}
