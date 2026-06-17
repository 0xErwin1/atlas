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

    // Links and wikilinks.
    '.cm-atlas-link': { color: 'var(--c-info)', cursor: 'pointer' },
    '.cm-atlas-wikilink': { color: 'var(--c-info)', cursor: 'pointer' },
    '.cm-atlas-wikilink:hover': { textDecoration: 'underline' },
    '.cm-atlas-wikilink-raw': { color: 'var(--c-info)' },
  },
  { dark: true },
);
