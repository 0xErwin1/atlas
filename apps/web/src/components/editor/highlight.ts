import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';

/**
 * Syntax highlighting for code inside the markdown editor, wired to the
 * `--c-syntax-*` design tokens (`src/theme/tokens.css`) so it follows the active
 * dark/light theme.
 *
 * Deliberately scoped to *code* tags (keyword, string, comment, number,
 * function, type, operator). Markdown structural tags (headings, emphasis,
 * links) are intentionally left out so they keep the `cm-atlas-*` live-preview
 * styling from `theme.ts`; a HighlightStyle only colours the tags it lists.
 */
export const atlasHighlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.keyword, color: 'var(--c-syntax-keyword)' },
    { tag: [t.string, t.special(t.string)], color: 'var(--c-syntax-string)' },
    {
      tag: [t.comment, t.lineComment, t.blockComment],
      color: 'var(--c-syntax-comment)',
      fontStyle: 'italic',
    },
    { tag: [t.number, t.bool, t.atom], color: 'var(--c-syntax-number)' },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: 'var(--c-syntax-function)' },
    { tag: [t.typeName, t.className, t.namespace], color: 'var(--c-syntax-type)' },
    { tag: [t.operator, t.derefOperator], color: 'var(--c-syntax-operator)' },
  ]),
);
