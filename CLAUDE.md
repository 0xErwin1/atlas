# CLAUDE.md — Atlas

Follow [`AGENTS.md`](AGENTS.md) for all project-specific guidance in this repository. It is the single source of truth for agent behavior, environment, commands, workspace layout, conventions, and architecture notes. For coding conventions and verification expectations, see [`CODE_STYLE.md`](CODE_STYLE.md); for system structure, see [`ARCHITECTURE.md`](ARCHITECTURE.md).

When instructions conflict, use this order of precedence:

1. The user's explicit request.
2. Higher-priority system or developer instructions.
3. `AGENTS.md` project instructions.
4. Existing codebase conventions.

Do not duplicate project rules here. Keep `AGENTS.md` as the single source of truth for repository-specific agent behavior.

## Hard rule — reuse, never duplicate

Do not reimplement or copy-paste UI (or any) patterns. Use the shared components/primitives (`Dropdown`, `Popover`, `ConfirmDialog`, `FormField`, `ExpandableRow`, …); the moment a pattern recurs, extract one component and have every call site use it. Duplicated markup/CSS/logic is a defect to remove, not extend. Full rule: `AGENTS.md` → Web frontend, and `CODE_STYLE.md` → TypeScript / Vue → Patterns.
