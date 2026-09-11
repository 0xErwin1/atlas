/**
 * Provider ids `reg5.rs` declares on `Experience::navigation_providers`
 * (E11-S8 design D-S8-4): acta renders its workspace/project navigation,
 * custos renders the admin surface. This list is the web side's copy of
 * that Rust-owned contract — it MUST be updated in the same PR that adds,
 * removes, or renames a `navigation_providers` id in `reg5.rs`.
 *
 * Kept as a single flat `as const` string array on purpose: the Rust test
 * `crates/atlas_server/tests/navigation_providers_web_copy.rs` parses this
 * exact literal shape (tolerant string-literal extraction, not a full
 * TS/JS parser) to assert this copy has not drifted from the registry's
 * real declarations. `navigationManifest.test.ts` audits
 * `NAVIGATION_MANIFEST` against this list in both directions
 * (INV-MANIFEST-BIDIRECTIONAL).
 */
export const DECLARED_NAVIGATION_PROVIDERS = ['acta.workspace', 'custos.admin'] as const;
