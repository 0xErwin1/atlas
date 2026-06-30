#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use atlas_api::dtos::{
    boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateReferenceRequest, CreateTaskRequest,
    },
    documents::{ConflictProblemDto, CreateDocumentRequest, DocumentDto, UpdateContentRequest},
    folders::CreateFolderRequest,
};
use atlas_client::AtlasClient;
use uuid::Uuid;

use crate::error::CliError;
use crate::output::OutputFormat;

use super::manifest::{Manifest, ManifestDocEntry, content_hash};
use super::plan::{DocumentOp, FolderOp, ImportPlan, LinkOp};

type FolderKey = (String, Option<Uuid>);

/// The maximum number of CAS update attempts before reporting a conflict error.
const MAX_CAS_RETRIES: usize = 3;

/// The execution action to take for a single document.
///
/// Determined by `decide_action` from manifest state, content hash, and server
/// existence — independently of the dry-run prediction in `DocAction`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DocDecision {
    /// Content is unchanged since last import — no API call needed.
    Skip,
    /// Document does not exist on the server — call `create_document`.
    Create,
    /// Document exists on the server — apply a CAS content update.
    Update,
}

/// Determines the per-document execution action from manifest and content state.
///
/// Manifest hit, same hash → Skip (content unchanged since last import).
/// Manifest hit, different hash → Update via CAS.
/// Manifest miss → always Create, letting the server assign a unique slug.
///
/// A manifest miss always creates, even when the predicted slug already exists on
/// the server. This ensures distinct vault files sharing the same title (e.g. ten
/// different `tasks.md` files under different folders) each become distinct
/// documents (`tasks`, `tasks-2`, …). The accepted trade-off: if the manifest
/// file is deleted, a re-import will create new documents rather than recognise
/// the originals.
pub(crate) fn decide_action(
    manifest_entry: Option<&ManifestDocEntry>,
    new_hash: &str,
) -> DocDecision {
    match manifest_entry {
        Some(entry) => {
            if entry.content_hash == new_hash {
                DocDecision::Skip
            } else {
                DocDecision::Update
            }
        }
        None => DocDecision::Create,
    }
}

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

/// Executes create, update, and skip operations for all document ops.
///
/// For each document the manifest is consulted first:
/// - Manifest hit, same hash → `[SKIP]`, no API call.
/// - Manifest hit, different hash → CAS `update_content` with the server's
///   current head revision (fetched immediately before the CAS attempt).
/// - Manifest miss → `create_document`; the server assigns a unique slug
///   (e.g. `tasks`, `tasks-2`) so distinct vault files sharing the same title
///   each become distinct documents. A manifest-loss re-import will create new
///   documents rather than recognising the originals.
///
/// The manifest is atomically saved after each successful operation so a
/// partial run can be resumed without data loss.
pub(crate) async fn execute_documents(
    client: &AtlasClient,
    ws: &str,
    project: &str,
    doc_ops: &[DocumentOp],
    manifest: &mut Manifest,
    manifest_path: &Path,
    output: OutputFormat,
) -> Result<(), CliError> {
    for op in doc_ops {
        let rel_key = op.rel_path.to_string_lossy().into_owned();
        let new_hash = content_hash(&op.content);
        let manifest_entry = manifest.documents.get(&rel_key).cloned();

        match decide_action(manifest_entry.as_ref(), &new_hash) {
            DocDecision::Skip => {
                let slug = manifest_entry
                    .as_ref()
                    .map(|e| e.slug.as_str())
                    .unwrap_or(&op.predicted_slug);
                emit_progress(output, "[SKIP]", slug);
            }

            DocDecision::Create => {
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

                let slug = dto.slug.ok_or_else(|| {
                    CliError::Validation(format!(
                        "server returned no slug for document '{}'; cannot persist manifest entry",
                        op.title
                    ))
                })?;

                manifest.documents.insert(
                    rel_key,
                    ManifestDocEntry {
                        slug: slug.clone(),
                        id: dto.id.to_string(),
                        content_hash: new_hash,
                    },
                );
                manifest.save(manifest_path)?;
                emit_progress(output, "[CREATE]", &slug);
            }

            DocDecision::Update => {
                let entry = manifest_entry.as_ref().ok_or_else(|| {
                    CliError::Validation(
                        "internal: Update decision without a manifest entry".into(),
                    )
                })?;

                let doc = client.get_document(ws, &entry.slug).await?;
                let slug = entry.slug.clone();

                let updated = cas_update_document(
                    client,
                    ws,
                    &slug,
                    op.content.clone(),
                    doc.head_revision_id,
                )
                .await?;

                let final_slug = updated.slug.unwrap_or_else(|| slug.clone());

                manifest.documents.insert(
                    rel_key,
                    ManifestDocEntry {
                        slug: final_slug.clone(),
                        id: updated.id.to_string(),
                        content_hash: new_hash,
                    },
                );
                manifest.save(manifest_path)?;
                emit_progress(output, "[UPDATE]", &final_slug);
            }
        }
    }

    Ok(())
}

/// Creates the "Roadmap" board (idempotent via manifest), ensures its three
/// standard columns exist, creates a task per epic, and attaches a `docs`
/// reference from each task back to its source document.
///
/// Phase order: boards → tasks → links. The manifest is saved after every
/// successful mutation so a partial run is resumable.
pub(crate) async fn execute_boards_and_tasks(
    client: &AtlasClient,
    ws: &str,
    project: &str,
    plan: &ImportPlan,
    manifest: &mut Manifest,
    manifest_path: &Path,
    output: OutputFormat,
) -> Result<(), CliError> {
    use atlas_client::ClientError;

    // Phase 1: Create boards; resolve and ensure required columns exist.
    // board_key → (board_uuid, column_name → column_uuid)
    let mut board_info: HashMap<String, (Uuid, HashMap<String, Uuid>)> = HashMap::new();

    for op in &plan.boards {
        let board_key = op.epic_rel.to_string_lossy().into_owned();

        let board_id = match manifest.boards.get(&board_key) {
            Some(id_str) => id_str.parse::<Uuid>().map_err(|e| {
                CliError::Validation(format!(
                    "invalid board UUID in manifest for '{board_key}': {e}"
                ))
            })?,
            None => {
                let dto = client
                    .create_board(
                        ws,
                        project,
                        CreateBoardRequest {
                            name: op.name.clone(),
                        },
                    )
                    .await?;
                let id = dto.id;
                manifest.boards.insert(board_key.clone(), id.to_string());
                manifest.save(manifest_path)?;
                emit_progress(output, "[BOARD_CREATE]", &op.name);
                id
            }
        };

        let column_map = ensure_board_columns(client, ws, board_id, &op.columns).await?;
        board_info.insert(board_key, (board_id, column_map));
    }

    // Phase 2: Create one task per epic, idempotent via manifest.tasks[rel_path].
    for op in &plan.tasks {
        let rel_key = op.rel_path.to_string_lossy().into_owned();

        if manifest.tasks.contains_key(&rel_key) {
            emit_progress(output, "[TASK_SKIP]", &op.title);
            continue;
        }

        let board_key = op.board_epic_rel.to_string_lossy().into_owned();
        let (board_id, column_map) = board_info.get(&board_key).ok_or_else(|| {
            CliError::Validation(format!(
                "board '{board_key}' not found; board must be created before tasks"
            ))
        })?;

        let column_id = *column_map.get(&op.column).ok_or_else(|| {
            CliError::Validation(format!(
                "column '{}' not found on board '{board_key}'",
                op.column
            ))
        })?;

        let description = if op.description.is_empty() {
            None
        } else {
            Some(op.description.clone())
        };

        let dto = client
            .create_task(
                ws,
                *board_id,
                CreateTaskRequest {
                    column_id,
                    title: op.title.clone(),
                    description,
                    properties: None,
                    before: None,
                    after: None,
                },
            )
            .await?;

        manifest.tasks.insert(rel_key, dto.readable_id.clone());
        manifest.save(manifest_path)?;
        emit_progress(output, "[TASK_CREATE]", &dto.readable_id);
    }

    // Phase 3: Link each task back to its source document via a "docs" reference.
    // Skip LinkOp::Parent — not handled in B3.
    for op in &plan.links {
        let (task_rel, source_doc_rel) = match op {
            LinkOp::Docs {
                task_rel,
                source_doc_rel,
            } => (task_rel, source_doc_rel),
            LinkOp::Parent { .. } => continue,
        };

        let task_key = task_rel.to_string_lossy().into_owned();
        let doc_key = source_doc_rel.to_string_lossy().into_owned();

        let readable_id = manifest.tasks.get(&task_key).ok_or_else(|| {
            CliError::Validation(format!(
                "task for '{task_key}' not in manifest; create tasks before linking"
            ))
        })?;

        let doc_entry = manifest.documents.get(&doc_key).ok_or_else(|| {
            CliError::Validation(format!(
                "document '{doc_key}' not in manifest; import documents before linking"
            ))
        })?;

        let doc_id = doc_entry.id.parse::<Uuid>().map_err(|e| {
            CliError::Validation(format!(
                "invalid document UUID in manifest for '{doc_key}': {e}"
            ))
        })?;

        match client
            .create_reference(
                ws,
                readable_id,
                CreateReferenceRequest {
                    kind: "docs".to_string(),
                    target_task_readable_id: None,
                    target_document_id: Some(doc_id),
                },
            )
            .await
        {
            Ok(_) => emit_progress(output, "[LINK]", readable_id),
            // A 409 means the reference already exists from a previous run — skip.
            Err(ClientError::Api(ref p)) if p.status == 409 => {}
            Err(e) => return Err(CliError::from(e)),
        }
    }

    Ok(())
}

/// Uploads binary attachments for their owning documents.
///
/// For each `AttachmentOp` in the plan:
/// - Resolves the owning document slug from the manifest (skips with a warning
///   when the owner has not been imported yet).
/// - Resolves the attachment file on disk via `resolve_attachment_path`; a
///   missing file is reported to stderr and the import continues (non-fatal).
/// - Checks for an already-uploaded attachment with the same filename to avoid
///   duplicates on re-import.
/// - Calls `upload_attachment`, treating a 409 response as an idempotent skip.
pub(crate) async fn execute_attachments(
    client: &AtlasClient,
    ws: &str,
    plan: &ImportPlan,
    manifest: &Manifest,
    vault_root: &Path,
    output: OutputFormat,
) -> Result<(), CliError> {
    use atlas_client::ClientError;

    for op in &plan.attachments {
        let owner_key = op.owner_rel.to_string_lossy().into_owned();

        let doc_entry = match manifest.documents.get(&owner_key) {
            Some(entry) => entry,
            None => {
                eprintln!(
                    "[WARN] attachment '{}': owning document '{}' not yet in manifest, skipping",
                    op.file_name, owner_key
                );
                continue;
            }
        };

        let file_bytes = match resolve_attachment_path(vault_root, &op.file_path, &op.owner_rel) {
            Some(path) => std::fs::read(&path)?,
            None => {
                eprintln!("[UNSUPPORTED] missing attachment: {}", op.file_name);
                continue;
            }
        };

        if attachment_exists_by_name(client, ws, &doc_entry.slug, &op.file_name).await? {
            emit_progress(output, "[ATTACH_SKIP]", &op.file_name);
            continue;
        }

        match client
            .upload_attachment(
                ws,
                &doc_entry.slug,
                &op.file_name,
                &op.content_type,
                file_bytes,
            )
            .await
        {
            Ok(_) => emit_progress(output, "[ATTACH]", &op.file_name),
            // A race between the existence check and the upload can produce 409;
            // treat it as a successful idempotent skip.
            Err(ClientError::Api(ref p)) if p.status == 409 => {
                emit_progress(output, "[ATTACH_SKIP]", &op.file_name);
            }
            Err(e) => return Err(CliError::from(e)),
        }
    }

    Ok(())
}

/// Resolves the on-disk path for an attachment file referenced via a vault embed.
///
/// Resolution order:
/// 1. `vault_root / file_path` — embed path treated as vault-relative.
/// 2. `vault_root / owner_dir / file_name` — relative to the owning note's directory.
/// 3. Recursive basename search anywhere under `vault_root`.
///
/// Returns `None` when the file cannot be found on disk.
pub(crate) fn resolve_attachment_path(
    vault_root: &Path,
    file_path: &Path,
    owner_rel: &Path,
) -> Option<PathBuf> {
    let candidate1 = vault_root.join(file_path);
    if candidate1.is_file() {
        return Some(candidate1);
    }

    let file_name = file_path.file_name()?;

    let owner_dir = owner_rel.parent().unwrap_or(Path::new(""));
    let candidate2 = vault_root.join(owner_dir).join(file_name);
    if candidate2 != candidate1 && candidate2.is_file() {
        return Some(candidate2);
    }

    basename_search(vault_root, file_name)
}

/// Walks `dir` recursively, returning the first path whose filename equals
/// `file_name`. Hidden directories (names starting with `.`) are skipped.
fn basename_search(dir: &Path, file_name: &OsStr) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if let Some(found) = basename_search(&path, file_name) {
                return Some(found);
            }
        } else if name == file_name {
            return Some(path);
        }
    }
    None
}

/// Returns `true` if a document attachment with `file_name` already exists,
/// paging through `list_attachments` until a match is found or the list ends.
async fn attachment_exists_by_name(
    client: &AtlasClient,
    ws: &str,
    slug: &str,
    file_name: &str,
) -> Result<bool, CliError> {
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .list_attachments(ws, slug, cursor.as_deref(), Some(100))
            .await?;

        if page.items.iter().any(|a| a.file_name == file_name) {
            return Ok(true);
        }

        if !page.has_more {
            return Ok(false);
        }

        cursor = page.next_cursor;
    }
}

/// Ensures all `desired` columns exist on `board_id`, creating any that are
/// missing. Returns a name → id map covering at least the desired set.
async fn ensure_board_columns(
    client: &AtlasClient,
    ws: &str,
    board_id: Uuid,
    desired: &[String],
) -> Result<HashMap<String, Uuid>, CliError> {
    let existing = client.list_columns(ws, board_id).await?;

    let mut column_map: HashMap<String, Uuid> =
        existing.into_iter().map(|c| (c.name, c.id)).collect();

    for name in desired {
        if !column_map.contains_key(name) {
            let col = client
                .create_column(
                    ws,
                    board_id,
                    CreateColumnRequest {
                        name: name.clone(),
                        color: None,
                        before: None,
                        after: None,
                    },
                )
                .await?;
            column_map.insert(col.name, col.id);
        }
    }

    Ok(column_map)
}

/// Applies a CAS content update, retrying on revision conflicts.
///
/// On each `ClientError::Conflict`, the server provides `current_revision_id` —
/// the head the client must rebase onto to avoid clobbering concurrent edits.
/// After `MAX_CAS_RETRIES` attempts without success the last conflict is
/// returned as `CliError::Conflict`.
async fn cas_update_document(
    client: &AtlasClient,
    ws: &str,
    slug: &str,
    content: String,
    initial_base: Uuid,
) -> Result<DocumentDto, CliError> {
    use atlas_client::ClientError;

    let mut base = initial_base;
    let mut last_conflict: Option<ConflictProblemDto> = None;

    for _ in 0..MAX_CAS_RETRIES {
        match client
            .update_content(
                ws,
                slug,
                UpdateContentRequest {
                    content: content.clone(),
                    base_revision_id: base,
                },
            )
            .await
        {
            Ok(dto) => return Ok(dto),
            Err(ClientError::Conflict(p)) => {
                // Rebase onto the server's current head to avoid clobbering concurrent edits.
                base = p.current_revision_id;
                last_conflict = Some(p);
            }
            Err(e) => return Err(CliError::from(e)),
        }
    }

    match last_conflict {
        Some(conflict) => Err(CliError::Conflict(Box::new(conflict))),
        None => Err(CliError::Validation(
            "CAS retry loop exhausted without executing any attempt".into(),
        )),
    }
}

/// Emits a per-operation progress line to stdout in Human output mode.
fn emit_progress(output: OutputFormat, kind: &str, target: &str) {
    if output == OutputFormat::Human {
        println!("{kind} {target}");
    }
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

    fn entry_with_hash(hash: &str) -> ManifestDocEntry {
        ManifestDocEntry {
            slug: "some-slug".into(),
            id: "some-uuid".into(),
            content_hash: hash.into(),
        }
    }

    // -- decide_action ---------------------------------------------------------

    #[test]
    fn decide_action_manifest_hit_same_hash_returns_skip() {
        let hash = "abc123";
        let entry = entry_with_hash(hash);
        assert_eq!(decide_action(Some(&entry), hash), DocDecision::Skip);
    }

    #[test]
    fn decide_action_manifest_hit_different_hash_returns_update() {
        let entry = entry_with_hash("old-hash");
        assert_eq!(decide_action(Some(&entry), "new-hash"), DocDecision::Update);
    }

    #[test]
    fn decide_action_no_manifest_always_returns_create() {
        assert_eq!(decide_action(None, "any-hash"), DocDecision::Create);
    }

    #[test]
    fn decide_action_no_manifest_always_creates_even_when_slug_exists_on_server() {
        // The caller no longer probes the server; a manifest miss is always Create
        // so distinct vault files with the same title each become distinct documents.
        assert_eq!(
            decide_action(None, "hash-for-second-tasks-file"),
            DocDecision::Create
        );
    }

    // -- CAS retry simulation --------------------------------------------------

    /// Simulates the CAS retry state machine against a sequence of pre-computed
    /// attempt outcomes, without touching the network.
    ///
    /// Each outcome is `Ok(())` (success) or `Err(current_revision_id)` (conflict).
    /// Returns `Ok(winning_base)` when any attempt within `MAX_CAS_RETRIES`
    /// succeeds, or `Err(last_conflict_revision_id)` when retries are exhausted.
    fn simulate_cas_retries(
        initial_base: Uuid,
        outcomes: &[Result<(), Uuid>],
    ) -> Result<Uuid, Uuid> {
        let mut base = initial_base;
        let mut last_conflict_id: Option<Uuid> = None;

        for outcome in outcomes.iter().take(MAX_CAS_RETRIES) {
            match outcome {
                Ok(()) => return Ok(base),
                Err(conflict_id) => {
                    last_conflict_id = Some(*conflict_id);
                    base = *conflict_id;
                }
            }
        }

        Err(last_conflict_id.unwrap_or(initial_base))
    }

    #[test]
    fn cas_immediate_success_returns_ok_with_initial_base() {
        let initial = uuid(1);
        let result = simulate_cas_retries(initial, &[Ok(())]);
        assert_eq!(result, Ok(initial));
    }

    #[test]
    fn cas_conflict_then_success_rebases_onto_conflict_revision() {
        let initial = uuid(1);
        let conflict_id = uuid(2);
        // First attempt conflicts; second succeeds with the rebased base.
        let result = simulate_cas_retries(initial, &[Err(conflict_id), Ok(())]);
        assert_eq!(result, Ok(conflict_id));
    }

    #[test]
    fn cas_three_conflicts_exhausts_retries_and_returns_last_conflict_id() {
        let initial = uuid(1);
        let conflict_a = uuid(2);
        let conflict_b = uuid(3);
        let conflict_c = uuid(4);
        let result = simulate_cas_retries(
            initial,
            &[Err(conflict_a), Err(conflict_b), Err(conflict_c)],
        );
        assert_eq!(result, Err(conflict_c));
    }

    #[test]
    fn cas_stops_at_max_retries_even_if_more_outcomes_provided() {
        // MAX_CAS_RETRIES = 3; the 4th Ok(()) must never be reached.
        let initial = uuid(1);
        let conflict_a = uuid(2);
        let conflict_b = uuid(3);
        let conflict_c = uuid(4);
        let result = simulate_cas_retries(
            initial,
            &[Err(conflict_a), Err(conflict_b), Err(conflict_c), Ok(())],
        );
        // Only 3 retries executed — all conflicts — so we get the exhaustion error.
        assert_eq!(result, Err(conflict_c));
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

    // -- resolve_attachment_path -----------------------------------------------

    #[test]
    fn resolve_attachment_path_vault_relative_hit() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();
        std::fs::write(vault_root.join("diagram.png"), b"data").unwrap();

        let result = resolve_attachment_path(
            vault_root,
            Path::new("diagram.png"),
            Path::new("notes/doc.md"),
        );
        assert_eq!(result, Some(vault_root.join("diagram.png")));
    }

    #[test]
    fn resolve_attachment_path_note_dir_hit() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();
        std::fs::create_dir(vault_root.join("notes")).unwrap();
        std::fs::write(vault_root.join("notes").join("diagram.png"), b"data").unwrap();

        // diagram.png does not exist at vault root; it is found via the owner's directory.
        let result = resolve_attachment_path(
            vault_root,
            Path::new("diagram.png"),
            Path::new("notes/doc.md"),
        );
        assert_eq!(result, Some(vault_root.join("notes").join("diagram.png")));
    }

    #[test]
    fn resolve_attachment_path_basename_search_hit() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();
        std::fs::create_dir(vault_root.join("assets")).unwrap();
        std::fs::write(vault_root.join("assets").join("diagram.png"), b"data").unwrap();

        // diagram.png is not at vault root nor in the owner's dir (which doesn't exist),
        // but the recursive basename search finds it under assets/.
        let result = resolve_attachment_path(
            vault_root,
            Path::new("diagram.png"),
            Path::new("notes/doc.md"),
        );
        assert_eq!(result, Some(vault_root.join("assets").join("diagram.png")));
    }

    #[test]
    fn resolve_attachment_path_not_found_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();

        let result =
            resolve_attachment_path(vault_root, Path::new("missing.png"), Path::new("doc.md"));
        assert!(result.is_none());
    }

    #[test]
    fn resolve_attachment_path_path_style_embed_vault_relative() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();
        std::fs::create_dir(vault_root.join("assets")).unwrap();
        std::fs::write(vault_root.join("assets").join("chart.pdf"), b"pdf").unwrap();

        // Embed was ![[assets/chart.pdf]] — path-style; the vault-relative join resolves it.
        let result = resolve_attachment_path(
            vault_root,
            Path::new("assets/chart.pdf"),
            Path::new("doc.md"),
        );
        assert_eq!(result, Some(vault_root.join("assets").join("chart.pdf")));
    }

    #[test]
    fn resolve_attachment_path_hidden_dirs_skipped_in_basename_search() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault_root = dir.path();
        // File only exists inside a hidden directory — must NOT be found.
        std::fs::create_dir(vault_root.join(".obsidian")).unwrap();
        std::fs::write(vault_root.join(".obsidian").join("diagram.png"), b"data").unwrap();

        let result =
            resolve_attachment_path(vault_root, Path::new("diagram.png"), Path::new("doc.md"));
        assert!(result.is_none());
    }
}
