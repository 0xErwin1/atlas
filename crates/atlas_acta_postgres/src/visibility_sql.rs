//! The V2 list visibility predicate as a SQL condition over the current
//! resource path of a listed Acta row (ACTA-AUTHZ-3, EVAL-5, PROV-3), applied
//! inside WHERE so LIMIT, cursors and counts only ever see visible rows.
//!
//! The condition reproduces the evaluator's `permits(path)`: a deny target
//! covering any node of the row's chain hides it; otherwise, among the grant
//! targets covering the chain, those at the nearest level and, within it,
//! the strongest specificity decide, and the row is visible when any of them
//! allows. Every segment text, tier and effect is a bound parameter.
//!
//! Each path builder yields a `text[]` of `<kind>::<id>` segments, root first,
//! shaped exactly as Acta's resource provider reports paths (D-E7-7):
//! `workspace/project?/folder*/document|board`, a task through its own board
//! (a subtask too), and a project directly under its workspace. A folder
//! chain is walked up to its root-most folder, whose project (if any) sits
//! between the workspace and the folders.

use atlas_core::ids::{SelectorSegment, Specificity};
use atlas_core::visibility::{ListVisibility, VisibilityGrant, VisibilityTarget};
use sea_orm::Value;

/// A WHERE condition and the values its placeholders bind, numbered from
/// the `first_bind` the caller passed.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlFragment {
    pub where_clause: String,
    pub binds: Vec<Value>,
}

/// The visibility of `kind` rows aliased `alias` as a WHERE condition.
/// Placeholders start at `$first_bind`.
pub fn predicate_to_sql(
    visibility: &ListVisibility,
    kind: VisibleKind,
    alias: &str,
    first_bind: usize,
) -> SqlFragment {
    let (grants, denies) = match visibility {
        ListVisibility::All => return constant("TRUE"),
        ListVisibility::Nothing => return constant("FALSE"),
        ListVisibility::Rules { grants, denies } => (grants, denies),
    };

    let coverable: Vec<&VisibilityGrant> = grants
        .iter()
        .filter(|grant| grant.target.product() == ACTA_PRODUCT)
        .collect();
    if coverable.is_empty() {
        return constant("FALSE");
    }

    let mut binder = Binder::starting_at(first_bind);
    let winner = winning_allow_sql(&coverable, &mut binder);
    let blocked: Vec<String> = denies
        .iter()
        .filter(|target| target.product() == ACTA_PRODUCT)
        .map(|target| format!("{} IS NOT NULL", level_sql(target, &mut binder)))
        .collect();

    let condition = if blocked.is_empty() {
        winner
    } else {
        format!("NOT ({}) AND {winner}", blocked.join(" OR "))
    };

    SqlFragment {
        where_clause: format!(
            "EXISTS (SELECT 1 FROM (SELECT {path} AS path) {PATH_ALIAS} WHERE {condition})",
            path = path_sql(kind, alias),
        ),
        binds: binder.binds,
    }
}

/// The product every Acta row path belongs to; targets of any other product
/// never cover one.
const ACTA_PRODUCT: &str = "acta";

/// The derived table carrying the row's path inside the condition.
const PATH_ALIAS: &str = "visibility_path";

fn constant(condition: &str) -> SqlFragment {
    SqlFragment {
        where_clause: condition.to_string(),
        binds: Vec::new(),
    }
}

/// Numbers placeholders from a starting index and keeps their values.
struct Binder {
    next: usize,
    binds: Vec<Value>,
}

impl Binder {
    fn starting_at(first: usize) -> Self {
        Self {
            next: first,
            binds: Vec::new(),
        }
    }

    fn bind(&mut self, value: impl Into<Value>) -> String {
        let placeholder = format!("${}", self.next);
        self.next += 1;
        self.binds.push(value.into());

        placeholder
    }
}

/// Whether the grants at the nearest covering level and strongest tier
/// include an allow. A grant's tier is its rank among the distinct
/// specificities of the rules, so equal specificities share a rank.
fn winning_allow_sql(grants: &[&VisibilityGrant], binder: &mut Binder) -> String {
    let mut tiers: Vec<Specificity> = grants
        .iter()
        .map(|grant| grant.target.specificity())
        .collect();
    tiers.sort();
    tiers.dedup();

    let rows: Vec<String> = grants
        .iter()
        .map(|grant| {
            let level = level_sql(&grant.target, binder);
            let tier = tiers
                .iter()
                .position(|tier| *tier == grant.target.specificity())
                .unwrap_or_default();
            let tier = binder.bind(i32::try_from(tier).unwrap_or(i32::MAX));
            let allow = binder.bind(grant.allow);

            format!("({level}, {tier}::int, {allow}::bool)")
        })
        .collect();

    format!(
        "COALESCE((SELECT bool_or(ranked.allow) FROM (\
             SELECT rule.allow, rank() OVER (ORDER BY rule.level, rule.tier DESC) AS place \
             FROM (VALUES {rows}) AS rule(level, tier, allow) \
             WHERE rule.level IS NOT NULL\
         ) ranked WHERE ranked.place = 1), FALSE)",
        rows = rows.join(", "),
    )
}

/// The chain level of the nearest node of the row's path that `target`
/// covers, as a `bigint` that is NULL when it covers none. Level 0 is the
/// row itself. The caller has checked the target's product.
fn level_sql(target: &VisibilityTarget, binder: &mut Binder) -> String {
    let path = format!("{PATH_ALIAS}.path");
    let length = format!("cardinality({path})::bigint");

    match target {
        VisibilityTarget::Ref(reference) => {
            let segment = binder.bind(format!("{}::{}", reference.kind(), reference.id()));

            format!(
                "(SELECT {length} - max(node.ordinal) \
                  FROM unnest({path}) WITH ORDINALITY AS node(element, ordinal) \
                  WHERE node.element = {segment})"
            )
        }
        VisibilityTarget::Path(exact) => {
            let segments: Vec<String> = exact.segments().map(ToString::to_string).collect();
            let depth = segments.len();
            let matched = prefix_matches(&path, segments.into_iter().map(Some), binder);

            format!("(CASE WHEN {length} >= {depth} {matched} THEN {length} - {depth} END)")
        }
        VisibilityTarget::Selector(selector) => {
            let segments = selector.segments().iter().map(|segment| match segment {
                SelectorSegment::Literal(literal) => Some(literal.to_string()),
                SelectorSegment::Any => None,
            });
            let explicit = selector.segments().len();
            let matched = prefix_matches(&path, segments, binder);

            if selector.is_open_ended() {
                let shortest = explicit.max(1);
                format!("(CASE WHEN {length} >= {shortest} {matched} THEN 0::bigint END)")
            } else {
                format!(
                    "(CASE WHEN {length} >= {explicit} {matched} THEN {length} - {explicit} END)"
                )
            }
        }
    }
}

/// `AND path[i] = $n` for every literal segment, 1-based; a wildcard
/// matches any element.
fn prefix_matches(
    path: &str,
    segments: impl Iterator<Item = Option<String>>,
    binder: &mut Binder,
) -> String {
    segments
        .enumerate()
        .filter_map(|(index, segment)| segment.map(|text| (index + 1, text)))
        .map(|(position, text)| format!(" AND {path}[{position}] = {}", binder.bind(text)))
        .collect()
}

/// The Acta resource kinds whose lists the V2 visibility predicate filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleKind {
    Project,
    Folder,
    Document,
    Board,
    Task,
}

/// The deepest folder chain the path walk follows; the V1 ancestor CTEs use
/// the same bound shape.
const MAX_FOLDER_DEPTH: usize = 64;

/// SQL for the path of the `kind` row aliased `alias`.
pub fn path_sql(kind: VisibleKind, alias: &str) -> String {
    match kind {
        VisibleKind::Project => format!(
            "ARRAY['workspace::' || {alias}.workspace_id::text, 'project::' || {alias}.id::text]"
        ),
        VisibleKind::Folder => folder_prefix_sql(&format!("{alias}.id")),
        VisibleKind::Document => format!(
            "(CASE WHEN {alias}.folder_id IS NOT NULL THEN {folder} \
                   ELSE {workspace_or_project} END) \
             || ARRAY['document::' || {alias}.id::text]",
            folder = folder_prefix_sql(&format!("{alias}.folder_id")),
            workspace_or_project = workspace_and_project_sql(alias),
        ),
        VisibleKind::Board => board_path_sql(alias),
        VisibleKind::Task => format!(
            "(SELECT {board} FROM acta.boards path_board WHERE path_board.id = {alias}.board_id) \
             || ARRAY['task::' || {alias}.id::text]",
            board = board_path_sql("path_board"),
        ),
    }
}

/// `workspace` then, when the row has one, `project`.
fn workspace_and_project_sql(alias: &str) -> String {
    format!(
        "ARRAY['workspace::' || {alias}.workspace_id::text] \
         || CASE WHEN {alias}.project_id IS NULL THEN ARRAY[]::text[] \
                 ELSE ARRAY['project::' || {alias}.project_id::text] END"
    )
}

/// A board sits under its folder when it has one, else under its project.
fn board_path_sql(alias: &str) -> String {
    format!(
        "(CASE WHEN {alias}.folder_id IS NOT NULL THEN {folder} \
               ELSE ARRAY['workspace::' || {alias}.workspace_id::text, \
                          'project::' || {alias}.project_id::text] END) \
         || ARRAY['board::' || {alias}.id::text]",
        folder = folder_prefix_sql(&format!("{alias}.folder_id")),
    )
}

/// The path of the folder `folder_id`: its workspace, the root-most
/// folder's project when set, then every folder from the root down to it.
/// A chain that never reaches a folder without a parent, through a cycle or
/// past the depth bound, has no path (NULL), so no rule covers the row, as
/// Acta's provider reports such a folder missing.
fn folder_prefix_sql(folder_id: &str) -> String {
    format!(
        "(WITH RECURSIVE path_up AS (\
             SELECT path_folder.id, path_folder.parent_folder_id, path_folder.project_id, \
                    path_folder.workspace_id, ARRAY[path_folder.id] AS seen, 1 AS depth \
             FROM acta.folders path_folder WHERE path_folder.id = {folder_id} \
             UNION ALL \
             SELECT path_parent.id, path_parent.parent_folder_id, path_parent.project_id, \
                    path_parent.workspace_id, path_up.seen || path_parent.id, path_up.depth + 1 \
             FROM acta.folders path_parent \
             JOIN path_up ON path_parent.id = path_up.parent_folder_id \
             WHERE NOT path_parent.id = ANY(path_up.seen) AND path_up.depth < {MAX_FOLDER_DEPTH}\
         ) \
         SELECT ARRAY['workspace::' || path_root.workspace_id::text] \
                || CASE WHEN path_root.project_id IS NULL THEN ARRAY[]::text[] \
                        ELSE ARRAY['project::' || path_root.project_id::text] END \
                || (SELECT array_agg('folder::' || path_chain.id::text ORDER BY path_chain.depth DESC) \
                    FROM path_up path_chain) \
         FROM path_up path_root WHERE path_root.parent_folder_id IS NULL)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE: &str = "acta::workspace::0190c3a4-5b6e-7f80-9a1b-2c3d4e5f6a7b";

    fn grant(target: VisibilityTarget, allow: bool) -> VisibilityGrant {
        VisibilityGrant { target, allow }
    }

    fn reference(raw: &str) -> VisibilityTarget {
        VisibilityTarget::Ref(raw.parse().unwrap())
    }

    fn texts(binds: &[Value]) -> Vec<String> {
        binds
            .iter()
            .filter_map(|value| match value {
                Value::String(Some(text)) => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn all_and_nothing_are_constant_conditions_without_binds() {
        let all = predicate_to_sql(&ListVisibility::All, VisibleKind::Document, "d", 3);
        let nothing = predicate_to_sql(&ListVisibility::Nothing, VisibleKind::Document, "d", 3);

        assert_eq!(all.where_clause, "TRUE");
        assert!(all.binds.is_empty());
        assert_eq!(nothing.where_clause, "FALSE");
        assert!(nothing.binds.is_empty());
    }

    #[test]
    fn rules_that_never_cover_an_acta_row_select_nothing() {
        let rules = ListVisibility::Rules {
            grants: vec![grant(reference("custos::group::g1"), true)],
            denies: vec![],
        };

        let fragment = predicate_to_sql(&rules, VisibleKind::Task, "t", 1);

        assert_eq!(fragment.where_clause, "FALSE");
        assert!(fragment.binds.is_empty());
    }

    #[test]
    fn a_grant_binds_its_segment_tier_and_effect_from_the_first_placeholder() {
        let rules = ListVisibility::Rules {
            grants: vec![grant(reference(WORKSPACE), true)],
            denies: vec![],
        };

        let fragment = predicate_to_sql(&rules, VisibleKind::Document, "d", 4);

        assert_eq!(
            texts(&fragment.binds),
            ["workspace::0190c3a4-5b6e-7f80-9a1b-2c3d4e5f6a7b"]
        );
        assert_eq!(fragment.binds.len(), 3);
        assert!(fragment.where_clause.contains("$4"));
        assert!(fragment.where_clause.contains("$6"));
        assert!(!fragment.where_clause.contains("$7"));
        assert!(!fragment.where_clause.contains("$3"));
        assert!(fragment.where_clause.contains("d.folder_id"));
        assert!(!fragment.where_clause.contains("0190c3a4"));
    }

    #[test]
    fn every_target_shape_binds_its_segments_and_denies_add_a_blocking_clause() {
        let rules = ListVisibility::Rules {
            grants: vec![
                grant(
                    VisibilityTarget::Path(format!("{WORKSPACE}/project::p1").parse().unwrap()),
                    true,
                ),
                grant(
                    VisibilityTarget::Selector(
                        format!("{WORKSPACE}/*/folder::f1/**").parse().unwrap(),
                    ),
                    false,
                ),
            ],
            denies: vec![reference("acta::folder::f2")],
        };

        let fragment = predicate_to_sql(&rules, VisibleKind::Folder, "f", 1);

        assert_eq!(
            texts(&fragment.binds),
            [
                "workspace::0190c3a4-5b6e-7f80-9a1b-2c3d4e5f6a7b",
                "project::p1",
                "workspace::0190c3a4-5b6e-7f80-9a1b-2c3d4e5f6a7b",
                "folder::f1",
                "folder::f2",
            ]
        );
        assert!(fragment.where_clause.contains("NOT ("));
        assert!(!fragment.where_clause.contains("folder::f"));
    }
}
