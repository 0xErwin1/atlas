import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { ChangeSpec, EditorState } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { paragraphSoftBreaks } from '@/lib/livePreview';

/**
 * Rewrites a hard-wrapped markdown document so each paragraph occupies a single
 * line, which is what makes it comfortable to edit: the editor soft-wraps to the
 * container, and a sentence no longer has to be shuffled across source lines by
 * hand. Reading mode reaches the same layout with decorations and never touches
 * the text (see `livePreviewExtension`); this is the deliberate, undoable rewrite
 * for the source itself.
 *
 * Hard breaks (a line ending in a backslash or two spaces) are meaningful and are
 * left alone, so a poem or an address keeps its shape.
 */

/**
 * Milliseconds of synchronous parse work allowed to bring the syntax tree up to
 * the whole document. The command is user-initiated and one-shot, so it can
 * afford a larger budget than the per-keystroke decoration passes.
 */
const PARSE_BUDGET_MS = 1000;

/**
 * Builds the joins for one document: one change per soft line break, each
 * replacing the break (plus the continuation line's indentation and blockquote
 * markers) with the single space CommonMark renders it as.
 *
 * Exported for unit testing the walk without a view.
 */
export function paragraphUnwrapChanges(
  state: EditorState,
  tree: ReturnType<typeof syntaxTree> = syntaxTree(state),
): ChangeSpec[] {
  const changes: ChangeSpec[] = [];

  tree.iterate({
    enter: (node) => {
      if (node.name !== 'Paragraph') return undefined;

      const source = state.doc.sliceString(node.from, node.to);
      for (const range of paragraphSoftBreaks(source, node.from, { consumeQuoteMarkers: true })) {
        changes.push({ from: range.from, to: range.to, insert: ' ' });
      }

      return false;
    },
  });

  return changes;
}

/**
 * Applies `paragraphUnwrapChanges` to the view as one undoable transaction.
 * Returns whether anything changed, so a caller can stay silent on a document
 * that is already unwrapped.
 */
export function unwrapParagraphs(view: EditorView): boolean {
  const state = view.state;
  const tree = ensureSyntaxTree(state, state.doc.length, PARSE_BUDGET_MS) ?? syntaxTree(state);
  const changes = paragraphUnwrapChanges(state, tree);

  if (changes.length === 0) return false;

  view.dispatch({ changes, userEvent: 'input.unwrap' });
  return true;
}
