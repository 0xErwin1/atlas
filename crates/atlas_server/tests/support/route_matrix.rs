#![allow(dead_code, clippy::panic)]

use std::sync::OnceLock;

use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

/// One live route the workspace registry declares (`v2-e3-s2` PR5). Replaces
/// the old hand-maintained route registry, retired in the same PR: the registry
/// itself is now the audited contract for the router (spec acceptance gate
/// 5), so the 401 sweep reads its `(method, path)` surface straight from
/// `atlas_core::registry::Registry` instead of a hand-maintained list.
pub(crate) struct RouteMatrixEntry {
    pub method: HttpMethod,
    pub path_template: String,
    /// `RouteDeclaration.is_public` (`v2-e3-s4`, D7): a direct registry
    /// read, not a recomputed union of `router_audit::*_route_paths()` — D7
    /// promoted that computation into the registry itself, so this field
    /// mirrors the registry's own value rather than re-deriving it.
    pub is_public: bool,
    /// The owning component's `ComponentEntry.identity.stable_id`
    /// (`v2-e3-s4` PR7, D10): sourced from the same `registry.entries()`
    /// loop `route_matrix()` already iterates, never a hand-typed per-path
    /// lookup table. Drives [`RouteMatrixEntry::namespaces`], so every
    /// namespace-scoped sweep checks a route at exactly its own two mounts.
    pub component: String,
}

impl RouteMatrixEntry {
    /// The concrete request path this entry is reachable at under `namespace`
    /// (`v2-e3-s4` PR5, D1, T5.8): every namespace-scoped sweep consumes this
    /// method instead of hand-concatenating `namespace` and `path_template`
    /// itself, reusing `router_audit::mounted_path` — the same primitive the
    /// production `app()` composition and every other namespace-aware test in
    /// this slice go through.
    pub(crate) fn mounted(&self, namespace: &str) -> String {
        atlas_server::router_audit::mounted_path(namespace, &self.path_template)
    }

    /// This entry's own two mounts (`v2-e3-s4` PR7, D10): `/api` and
    /// `/api/v2/<component>`, wrapping `router_audit::namespaces_for` with
    /// this entry's own `component` — the per-component replacement for the
    /// former flat `router_audit::NAMESPACES` constant.
    pub(crate) fn namespaces(&self) -> [String; 2] {
        atlas_server::router_audit::namespaces_for(&self.component)
    }

    /// This entry's path, mounted under the suite's current default
    /// namespace (`v2-e3-s5` design D5): delegates to
    /// `support::path::api_path`, so an entry-based lookup and a
    /// literal-based `api_path` call resolve through the same
    /// `DEFAULT_NAMESPACE_INDEX`. Additive to [`RouteMatrixEntry::namespaces`]
    /// — a sweep that legitimately exercises both mounts keeps calling
    /// `namespaces()` directly and MUST NOT be rewritten to this method.
    pub(crate) fn mounted_default(&self) -> String {
        crate::support::path::api_path(&self.component, &self.path_template)
    }
}

/// One `route_matrix()` snapshot, built once per test binary (`v2-e3-s5`
/// design D2/R7): `component_declares` runs on every `api_path` call across
/// 249+166 migrated call sites, and rebuilding the live registry that often
/// would make each of them pay `route_matrix()`'s registry-build cost.
static MATRIX: OnceLock<Vec<RouteMatrixEntry>> = OnceLock::new();

fn matrix_once() -> &'static [RouteMatrixEntry] {
    MATRIX.get_or_init(route_matrix)
}

/// Whether `component` owns a route matching `relative` (`v2-e3-s5` design
/// D2): the assertion `api_path` needs before trusting a component argument
/// that `namespaces_for(component)[DEFAULT_NAMESPACE_INDEX]` would otherwise
/// discard. Uses `.any()` over the cached matrix with the existing
/// `template_matches` matcher, never [`find_by_concrete_path`] — this check
/// has no need to disambiguate between two matching templates, only to
/// confirm at least one owning entry exists.
///
/// A [`atlas_server::router_audit::ROOT_LEVEL_PATHS`] member is declared for
/// every component: `mounted_path` serves it unprefixed regardless of which
/// component asked (spec requirement "root-level paths are exempt through
/// `mounted_path`, not through a test-side special case"), so ownership
/// validation would otherwise wrongly reject a root-level request made
/// through an unrelated component's call site.
pub(crate) fn component_declares(component: &str, relative: &str) -> bool {
    if atlas_server::router_audit::ROOT_LEVEL_PATHS.contains(&relative) {
        return true;
    }

    matrix_once().iter().any(|entry| {
        entry.component == component && template_matches(&entry.path_template, relative)
    })
}

/// Builds the full `(method, path)` surface from the live REG-5 registry,
/// classified public/protected by each route's own `is_public` field.
pub(crate) fn route_matrix() -> Vec<RouteMatrixEntry> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    registry
        .entries()
        .iter()
        .flat_map(|entry| {
            let component = entry.identity.stable_id.as_str().to_string();
            entry.api.routes.iter().map(move |route| RouteMatrixEntry {
                method: route.method,
                path_template: route.path.as_str().to_string(),
                is_public: route.is_public,
                component: component.clone(),
            })
        })
        .collect()
}

/// Resolves a concrete request path (`/workspaces/ws-slug/tasks/ATL-1`) to
/// the one live `route_matrix()` entry whose template matches it, so a
/// caller that only has the path it is about to request never compares it
/// verbatim against a `{placeholder}` template.
///
/// Matching is segment by segment: same segment count, literal segments
/// equal, a `{...}` segment matches any non-empty concrete segment. When a
/// literal template and a placeholder template both match (`/a/literal` vs.
/// `/a/{x}`), the literal wins. Panics naming the path and every candidate
/// when nothing matches or the tie cannot be broken.
pub(crate) fn find_by_concrete_path(method: HttpMethod, concrete: &str) -> RouteMatrixEntry {
    resolve_concrete_path(route_matrix(), method, concrete)
}

fn resolve_concrete_path(
    entries: Vec<RouteMatrixEntry>,
    method: HttpMethod,
    concrete: &str,
) -> RouteMatrixEntry {
    let candidates: Vec<RouteMatrixEntry> = entries
        .into_iter()
        .filter(|entry| entry.method == method && template_matches(&entry.path_template, concrete))
        .collect();

    if candidates.is_empty() {
        panic!("no live route_matrix() entry matches {method} {concrete}");
    }

    let fewest_placeholders = candidates
        .iter()
        .map(|entry| placeholder_count(&entry.path_template))
        .min()
        .expect("candidates is non-empty");
    let mut best: Vec<RouteMatrixEntry> = candidates
        .into_iter()
        .filter(|entry| placeholder_count(&entry.path_template) == fewest_placeholders)
        .collect();

    if best.len() > 1 {
        let templates: Vec<&str> = best
            .iter()
            .map(|entry| entry.path_template.as_str())
            .collect();
        panic!("{method} {concrete} is ambiguous between route_matrix() templates {templates:?}");
    }

    best.remove(0)
}

fn template_matches(template: &str, concrete: &str) -> bool {
    let template_segments: Vec<&str> = template.split('/').collect();
    let concrete_segments: Vec<&str> = concrete.split('/').collect();

    template_segments.len() == concrete_segments.len()
        && template_segments
            .iter()
            .zip(&concrete_segments)
            .all(|(t, c)| {
                if is_placeholder(t) {
                    !c.is_empty()
                } else {
                    t == c
                }
            })
}

fn is_placeholder(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

fn placeholder_count(template: &str) -> usize {
    template
        .split('/')
        .filter(|segment| is_placeholder(segment))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(method: HttpMethod, path_template: &str) -> RouteMatrixEntry {
        RouteMatrixEntry {
            method,
            path_template: path_template.to_string(),
            is_public: false,
            component: "acta".to_string(),
        }
    }

    fn fixture() -> Vec<RouteMatrixEntry> {
        vec![
            entry(HttpMethod::Get, "/workspaces/{ws}/tasks/{readable_id}"),
            entry(HttpMethod::Get, "/workspaces/{ws}/tasks/views"),
            entry(HttpMethod::Post, "/workspaces/{ws}/tasks/{readable_id}"),
        ]
    }

    #[test]
    fn parameterized_path_resolves_to_its_template() {
        let found =
            resolve_concrete_path(fixture(), HttpMethod::Get, "/workspaces/ws-1/tasks/ATL-1");

        assert_eq!(found.path_template, "/workspaces/{ws}/tasks/{readable_id}");
    }

    #[test]
    fn literal_segment_wins_over_placeholder() {
        let found =
            resolve_concrete_path(fixture(), HttpMethod::Get, "/workspaces/ws-1/tasks/views");

        assert_eq!(found.path_template, "/workspaces/{ws}/tasks/views");
    }

    #[test]
    #[should_panic(
        expected = "no live route_matrix() entry matches DELETE /workspaces/ws-1/tasks/ATL-1"
    )]
    fn unknown_path_panics() {
        resolve_concrete_path(
            fixture(),
            HttpMethod::Delete,
            "/workspaces/ws-1/tasks/ATL-1",
        );
    }

    #[test]
    fn component_declares_is_true_for_a_real_pair_from_the_live_registry() {
        assert!(component_declares("acta", "/workspaces/{ws}/tasks"));
    }

    #[test]
    fn component_declares_is_false_for_a_wrong_component() {
        assert!(!component_declares("custos", "/workspaces/{ws}/tasks"));
    }

    #[test]
    fn component_declares_is_false_for_a_nonexistent_relative_path() {
        assert!(!component_declares("acta", "/no/such/route"));
    }

    #[test]
    fn mounted_default_matches_api_path_for_every_live_entry() {
        for entry in route_matrix() {
            assert_eq!(
                entry.mounted_default(),
                crate::support::path::api_path(&entry.component, &entry.path_template)
            );
        }
    }
}
