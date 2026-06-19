import { EditorView } from '@codemirror/view';

/**
 * CodeMirror 6 theme for the Atlas markdown editor, wired to the Ayu-dark design
 * tokens (`src/theme/tokens.css`). The editor is a document surface, not a form
 * field: transparent background, no border, no focus ring.
 *
 * Live-preview construct classes (`cm-atlas-*`) are emitted by
 * `livePreviewExtension.ts`; their visual styling lives here so the whole editor
 * appearance is defined in one place.
 */
export const atlasMarkdownTheme = EditorView.theme(
  {
    '&': {
      color: 'var(--c-foreground)',
      backgroundColor: 'transparent',
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-lg)',
    },
    '&.cm-focused': {
      outline: 'none',
    },
    '.cm-scroller': {
      fontFamily: 'var(--font-mono)',
      lineHeight: 'var(--lh-relaxed)',
    },
    '.cm-content': {
      caretColor: 'var(--c-primary)',
      padding: '0',
    },
    '.cm-line': {
      padding: '0',
    },
    '&.cm-editor': {
      backgroundColor: 'transparent',
    },
    '.cm-cursor, .cm-dropCursor': {
      borderLeftColor: 'var(--c-primary)',
    },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'var(--c-input)',
    },
    '.cm-gutters': {
      display: 'none',
    },
    '.cm-placeholder': {
      color: 'var(--c-muted)',
    },

    // Headings: size by level, bold, foreground.
    '.cm-atlas-h1': { fontSize: '1.8em', fontWeight: 'var(--fw-bold)', lineHeight: '1.3' },
    '.cm-atlas-h2': { fontSize: '1.55em', fontWeight: 'var(--fw-bold)', lineHeight: '1.3' },
    '.cm-atlas-h3': { fontSize: '1.35em', fontWeight: 'var(--fw-bold)', lineHeight: '1.3' },
    '.cm-atlas-h4': { fontSize: '1.2em', fontWeight: 'var(--fw-bold)', lineHeight: '1.3' },
    '.cm-atlas-h5': { fontSize: '1.08em', fontWeight: 'var(--fw-bold)', lineHeight: '1.3' },
    '.cm-atlas-h6': { fontSize: '1em', fontWeight: 'var(--fw-bold)', color: 'var(--c-muted)' },

    // Inline emphasis.
    '.cm-atlas-strong': { fontWeight: 'var(--fw-bold)', color: 'var(--c-foreground)' },
    '.cm-atlas-em': { fontStyle: 'italic' },
    '.cm-atlas-strike': { textDecoration: 'line-through', color: 'var(--c-muted)' },

    // Inline code.
    '.cm-atlas-code': {
      fontFamily: 'var(--font-mono)',
      backgroundColor: 'var(--c-input)',
      borderRadius: 'var(--r-sm)',
      padding: '1px 4px',
    },

    // Fenced code block lines.
    '.cm-atlas-fenced': {
      backgroundColor: 'var(--c-raised)',
      fontFamily: 'var(--font-mono)',
    },
    '.cm-atlas-fenced:first-of-type': {
      borderTop: '1px solid var(--c-border)',
    },

    // Language badge shown in place of the opening ```lang fence off active line.
    '.cm-atlas-lang': {
      display: 'inline-block',
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-xs)',
      textTransform: 'uppercase',
      letterSpacing: '0.05em',
      color: 'var(--c-muted)',
    },

    // Rendered GFM table, in place of the raw pipe markdown off the active block.
    '.cm-atlas-table-wrap': {
      overflowX: 'auto',
      margin: '0.2em 0',
      cursor: 'text',
    },
    '.cm-atlas-table': {
      borderCollapse: 'collapse',
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-base)',
    },
    '.cm-atlas-table th, .cm-atlas-table td': {
      border: '1px solid var(--c-border)',
      padding: '4px 10px',
      textAlign: 'left',
    },
    '.cm-atlas-table th': {
      backgroundColor: 'var(--c-raised)',
      fontWeight: 'var(--fw-semibold)',
      color: 'var(--c-foreground)',
    },

    // Blockquote.
    '.cm-atlas-quote': {
      borderLeft: '3px solid var(--c-border)',
      paddingLeft: '12px',
      color: 'var(--c-muted)',
    },

    // Horizontal rule line.
    '.cm-atlas-hr': {
      borderBottom: '1px solid var(--c-border)',
      color: 'var(--c-muted)',
    },

    // List item line. The raw bullet marker (`-`/`*`/`+`) is replaced by a `•`
    // widget off the active line; ordered markers are left as content.
    '.cm-atlas-listitem': {},
    '.cm-atlas-bullet': { color: 'var(--c-muted)' },

    // Task-list checkbox, rendered in place of the raw `[ ]`/`[x]` marker.
    '.cm-atlas-checkbox': {
      appearance: 'none',
      width: '1em',
      height: '1em',
      margin: '0 0.4em 0 0',
      verticalAlign: '-0.12em',
      border: '1.5px solid var(--c-muted)',
      borderRadius: 'var(--r-sm)',
      backgroundColor: 'transparent',
      cursor: 'pointer',
      position: 'relative',
    },
    '.cm-atlas-checkbox:checked': {
      backgroundColor: 'var(--c-primary)',
      borderColor: 'var(--c-primary)',
    },
    '.cm-atlas-checkbox:checked::after': {
      content: '""',
      position: 'absolute',
      left: '0.28em',
      top: '0.1em',
      width: '0.25em',
      height: '0.5em',
      border: 'solid var(--c-primary-fg)',
      borderWidth: '0 2px 2px 0',
      transform: 'rotate(45deg)',
    },

    // Rendered image, in place of the raw ![alt](url) off active line.
    '.cm-atlas-img': {
      display: 'inline-block',
      maxWidth: '100%',
      height: 'auto',
      borderRadius: 'var(--r-lg)',
      verticalAlign: 'top',
    },

    // Links and wikilinks.
    '.cm-atlas-link': { color: 'var(--c-info)', cursor: 'pointer' },
    '.cm-atlas-wikilink': { color: 'var(--c-info)', cursor: 'pointer' },
    '.cm-atlas-wikilink:hover': { textDecoration: 'underline' },
    '.cm-atlas-wikilink-raw': { color: 'var(--c-info)' },
  },
  { dark: true },
);
