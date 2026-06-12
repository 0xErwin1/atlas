#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_domain::ids::{ApiKeyId, BoardId, DocumentId, ProjectId, TaskId, UserId};
use atlas_domain::entities::boards_tasks::ReferenceKind;
use atlas_domain::entities::identity::MemberRole;
use atlas_domain::permissions::{
    ChainSegment, Principal, ResolutionInput, ResourceChain, ResourceRef, ResourceRole, ShareDenied,
    Visibility, VisibilityRole, authorize_share, resolve, validate_reference,
};
use atlas_domain::error::DomainError;

fn user_principal() -> Principal {
    Principal::User(UserId::new())
}

fn api_key_principal() -> Principal {
    Principal::ApiKey(ApiKeyId::new())
}

fn project_ref() -> ResourceRef {
    ResourceRef::Project(ProjectId::new())
}

fn board_ref() -> ResourceRef {
    ResourceRef::Board(BoardId::new())
}

fn doc_ref() -> ResourceRef {
    ResourceRef::Document(DocumentId::new())
}

// ——— resolve() tests ———

#[test]
fn implicit_admin_owner_membership() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Owner),
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Admin));
}

#[test]
fn implicit_admin_admin_membership() {
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: project_ref(), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Admin),
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Admin));
}

#[test]
fn inheritance_down_chain() {
    let pid = ProjectId::new();
    let bid = BoardId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Board(bid), visibility: None },
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[(ResourceRef::Project(pid), ResourceRole::Editor)],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn most_specific_wins() {
    let pid = ProjectId::new();
    let did = DocumentId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Document(did), visibility: None },
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[
            (ResourceRef::Project(pid), ResourceRole::Viewer),
            (ResourceRef::Document(did), ResourceRole::Editor),
        ],
    };
    // document grant (more specific) wins over project grant
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn max_at_equal_specificity() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment {
                resource: ResourceRef::Project(pid),
                visibility: Some(Visibility::Workspace(VisibilityRole::Editor)),
            },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    // explicit grant says Viewer, visibility says Editor (both same segment) → max = Editor
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[(ResourceRef::Project(pid), ResourceRole::Viewer)],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn visibility_workspace_contributes_for_member_user() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment {
                resource: ResourceRef::Project(pid),
                visibility: Some(Visibility::Workspace(VisibilityRole::Editor)),
            },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn visibility_does_not_contribute_for_api_key() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment {
                resource: ResourceRef::Project(pid),
                visibility: Some(Visibility::Workspace(VisibilityRole::Editor)),
            },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    // ApiKey has no explicit grant, visibility doesn't apply to agents
    let input = ResolutionInput {
        principal: &api_key_principal(),
        membership: None,
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), None);
}

#[test]
fn private_visibility_gives_no_role() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment {
                resource: ResourceRef::Project(pid),
                visibility: Some(Visibility::Private),
            },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), None);
}

#[test]
fn workspace_scope_grant_is_least_specific() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[(ResourceRef::Workspace, ResourceRole::Editor)],
    };
    // Only workspace-scope grant available → Editor
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn default_deny_no_applicable_grant() {
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: project_ref(), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &user_principal(),
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &[],
    };
    assert_eq!(resolve(&input), None);
}

#[test]
fn agent_cap_admin_grant_capped_to_editor() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &api_key_principal(),
        membership: None,
        chain: &chain,
        grants: &[(ResourceRef::Project(pid), ResourceRole::Admin)],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

#[test]
fn agent_cap_editor_grant_stays_editor() {
    let pid = ProjectId::new();
    let chain = ResourceChain {
        segments: vec![
            ChainSegment { resource: ResourceRef::Project(pid), visibility: None },
            ChainSegment { resource: ResourceRef::Workspace, visibility: None },
        ],
    };
    let input = ResolutionInput {
        principal: &api_key_principal(),
        membership: None,
        chain: &chain,
        grants: &[(ResourceRef::Project(pid), ResourceRole::Editor)],
    };
    assert_eq!(resolve(&input), Some(ResourceRole::Editor));
}

// ——— authorize_share() tests ———

#[test]
fn editor_can_grant_viewer() {
    let result = authorize_share(
        &user_principal(),
        ResourceRole::Editor,
        ResourceRole::Viewer,
    );
    assert!(result.is_ok());
}

#[test]
fn editor_can_grant_editor() {
    let result = authorize_share(
        &user_principal(),
        ResourceRole::Editor,
        ResourceRole::Editor,
    );
    assert!(result.is_ok());
}

#[test]
fn editor_cannot_grant_admin() {
    let result = authorize_share(
        &user_principal(),
        ResourceRole::Editor,
        ResourceRole::Admin,
    );
    assert_eq!(result, Err(ShareDenied::RoleExceedsGrantors));
}

#[test]
fn admin_can_grant_admin() {
    let result = authorize_share(
        &user_principal(),
        ResourceRole::Admin,
        ResourceRole::Admin,
    );
    assert!(result.is_ok());
}

#[test]
fn agent_never_manages_grants() {
    let result = authorize_share(
        &api_key_principal(),
        ResourceRole::Editor,
        ResourceRole::Viewer,
    );
    assert_eq!(result, Err(ShareDenied::AgentsNeverManageGrants));
}

#[test]
fn insufficient_role_to_share_below_editor() {
    let result = authorize_share(
        &user_principal(),
        ResourceRole::Viewer,
        ResourceRole::Viewer,
    );
    assert_eq!(result, Err(ShareDenied::InsufficientRoleToShare));
}

// ——— validate_reference() tests ———

#[test]
fn blocks_with_task_id_is_valid() {
    let tid = TaskId::new();
    let result = validate_reference(ReferenceKind::Blocks, Some(tid), None);
    assert!(result.is_ok());
}

#[test]
fn spec_with_document_id_is_valid() {
    let did = DocumentId::new();
    let result = validate_reference(ReferenceKind::Spec, None, Some(did));
    assert!(result.is_ok());
}

#[test]
fn blocks_with_document_id_is_invalid() {
    let did = DocumentId::new();
    let result = validate_reference(ReferenceKind::Blocks, None, Some(did));
    assert!(matches!(result, Err(DomainError::InvalidInput { .. })));
}

#[test]
fn spec_with_task_id_is_invalid() {
    let tid = TaskId::new();
    let result = validate_reference(ReferenceKind::Spec, Some(tid), None);
    assert!(matches!(result, Err(DomainError::InvalidInput { .. })));
}

#[test]
fn both_targets_is_invalid() {
    let tid = TaskId::new();
    let did = DocumentId::new();
    let result = validate_reference(ReferenceKind::Blocks, Some(tid), Some(did));
    assert!(matches!(result, Err(DomainError::InvalidInput { .. })));
}

#[test]
fn neither_target_is_invalid() {
    let result = validate_reference(ReferenceKind::Relates, None, None);
    assert!(matches!(result, Err(DomainError::InvalidInput { .. })));
}

#[test]
fn relates_with_task_id_is_valid() {
    let tid = TaskId::new();
    let result = validate_reference(ReferenceKind::Relates, Some(tid), None);
    assert!(result.is_ok());
}

#[test]
fn parent_with_task_id_is_valid() {
    let tid = TaskId::new();
    let result = validate_reference(ReferenceKind::Parent, Some(tid), None);
    assert!(result.is_ok());
}
