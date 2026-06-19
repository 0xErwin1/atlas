import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { type Range, RangeSetBuilder } from '@codemirror/state';
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import {
  computeActiveLines,
  fenceLanguage,
  type LineRange,
  parseImage,
  type SelectionRange,
  taskMarkerChecked,
} from '@/lib/livePreview';
import { parseWikilinkInner, type WikilinkRef } from '@/lib/wikilink';

/**
 * Lezer syntax node, derived from `Tree.resolve` so we do not depend on the
 * transitive `@lezer/common` package being hoisted into node_modules.
 */
type SyntaxNode = ReturnType<ReturnType<typeof syntaxTree>['resolve']>;

/**
 * Obsidian-style "Live Preview" decorations for the CodeMirror 6 markdown editor.
 *
 * The document stays as raw markdown (source of truth). This ViewPlugin walks the
 * Lezer markdown syntax tree over the visible ranges and:
 *   - HIDES syntax markers (`#`, `**`, backticks, `~~`, link brackets) and styles
 *     the rendered content, WHEN the marker's line is NOT touched by the selection;
 *   - REVEALS the raw markers (no replace decoration), styling preserved, WHEN the
 *     line IS active, so the user can edit them (delete a `#` to demote a heading).
 *
 * Wikilinks (`[[Title]]`) are not part of the Lezer markdown grammar, so they are
 * decorated by a separate regex pass with the same reveal-on-active-line rule.
 *
 * The active-line decision is delegated to the pure `computeActiveLines` helper so
 * it stays unit-testable without a DOM.
 */

export interface LivePreviewCallbacks {
  /** Called when a rendered (collapsed) wikilink is clicked. */
  onWikilinkClick: (ref: WikilinkRef) => void;
}

export interface LivePreviewOptions {
  /**
   * When true (edit mode), syntax markers on the line the cursor/selection
   * touches are revealed raw so they can be edited. When false (preview / reading
   * mode), no line is ever treated as active: every marker stays hidden and the
   * document reads as fully rendered, like Obsidian's reading view.
   */
  reveal: boolean;
  /**
   * Live id → current-title map for id-bound wikilinks. A rendered link shows the
   * resolved title when present, falling back to the snapshot title in the text.
   */
  titles?: Record<string, string>;
}

const WIKILINK_RE = /\[\[([^[\]\n]+)\]\]/g;

/**
 * Widget that renders a collapsed wikilink as clickable text. The raw
 * `[[Title]]` is replaced by this when the link's line is not active.
 */
class WikilinkWidget extends WidgetType {
  constructor(
    private readonly ref: WikilinkRef,
    private readonly display: string,
    private readonly onClick: (ref: WikilinkRef) => void,
  ) {
    super();
  }

  eq(other: WikilinkWidget): boolean {
    return (
      other.ref.id === this.ref.id && other.ref.title === this.ref.title && other.display === this.display
    );
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-atlas-wikilink';
    span.textContent = this.display;
    span.addEventListener('mousedown', (event) => {
      event.preventDefault();
      this.onClick(this.ref);
    });
    return span;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

/**
 * Widget that renders a list bullet in place of the raw `-`/`*`/`+` marker, so an
 * off-active bullet line reads as `• item` while the document keeps the markdown
 * marker. Ordered-list markers (`1.`) are meaningful content and never replaced.
 */
class BulletWidget extends WidgetType {
  eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-atlas-bullet';
    span.textContent = '•';
    return span;
  }
}

/**
 * Widget that renders a GFM task marker (`[ ]`/`[x]`) as a real checkbox. Click
 * toggles the underlying `[ ]`↔`[x]` in the source, unless the editor is
 * read-only (preview mode), where the box reflects state without mutating.
 */
class CheckboxWidget extends WidgetType {
  constructor(
    private readonly checked: boolean,
    private readonly from: number,
  ) {
    super();
  }

  eq(other: CheckboxWidget): boolean {
    return other.checked === this.checked && other.from === this.from;
  }

  toDOM(view: EditorView): HTMLElement {
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.className = 'cm-atlas-checkbox';
    box.checked = this.checked;

    // Keep the click from moving the caret into the line (which would reveal the
    // raw marker); the toggle is an explicit document edit instead.
    box.addEventListener('mousedown', (event) => event.preventDefault());
    box.addEventListener('click', (event) => {
      event.preventDefault();
      if (view.state.readOnly) {
        box.checked = this.checked;
        return;
      }
      view.dispatch({
        changes: { from: this.from, to: this.from + 3, insert: this.checked ? '[ ]' : '[x]' },
      });
    });

    return box;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

/**
 * Widget that renders the fenced-code language label in place of the opening
 * ```` ```lang ```` marker, so an off-active code block reads like GitHub: a small
 * language tag instead of the raw backticks.
 */
class LangBadgeWidget extends WidgetType {
  constructor(private readonly lang: string) {
    super();
  }

  eq(other: LangBadgeWidget): boolean {
    return other.lang === this.lang;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-atlas-lang';
    span.textContent = this.lang;
    return span;
  }
}

/**
 * Widget that renders a markdown image `![alt](url)` as an actual `<img>` in place
 * of the raw markdown, off the active line. The source markdown is restored when
 * the cursor enters the line, keeping it editable.
 */
class ImageWidget extends WidgetType {
  constructor(
    private readonly url: string,
    private readonly alt: string,
  ) {
    super();
  }

  eq(other: ImageWidget): boolean {
    return other.url === this.url && other.alt === this.alt;
  }

  toDOM(): HTMLElement {
    const img = document.createElement('img');
    img.className = 'cm-atlas-img';
    img.src = this.url;
    img.alt = this.alt;
    return img;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

const hideDeco = Decoration.replace({});

function lineRangesFor(view: EditorView): LineRange[] {
  const out: LineRange[] = [];
  const doc = view.state.doc;

  for (let n = 1; n <= doc.lines; n += 1) {
    const line = doc.line(n);
    out.push({ number: line.number, from: line.from, to: line.to });
  }

  return out;
}

function selectionRangesFor(view: EditorView): SelectionRange[] {
  return view.state.selection.ranges.map((r) => ({ from: r.from, to: r.to }));
}

function lineNumberAt(view: EditorView, pos: number): number {
  return view.state.doc.lineAt(pos).number;
}

/**
 * Builds the full decoration set for the current view state.
 *
 * Decorations are collected unsorted into an array, then sorted by `from` (and by
 * startSide) before being fed to a RangeSetBuilder, because CM6 requires
 * decorations added in document order. Line decorations and mark/replace
 * decorations are interleaved by position.
 */
function buildDecorations(
  view: EditorView,
  callbacks: LivePreviewCallbacks,
  reveal: boolean,
  titles: Record<string, string>,
): DecorationSet {
  const activeLines = reveal
    ? computeActiveLines(lineRangesFor(view), selectionRangesFor(view))
    : new Set<number>();
  const decos: Range<Decoration>[] = [];

  for (const { from, to } of view.visibleRanges) {
    decorateSyntaxTree(view, from, to, activeLines, decos);
    decorateWikilinks(view, from, to, activeLines, callbacks, titles, decos);
  }

  decos.sort((a, b) => a.from - b.from || a.value.startSide - b.value.startSide);

  const builder = new RangeSetBuilder<Decoration>();
  for (const deco of decos) builder.add(deco.from, deco.to, deco.value);
  return builder.finish();
}

/**
 * Walks the Lezer markdown tree over [from, to] and pushes decorations for every
 * supported construct. The reveal-on-active-line rule is applied per construct:
 * markers on an active line are left raw (only the content styling is applied),
 * markers elsewhere are collapsed with a replace decoration.
 */
function decorateSyntaxTree(
  view: EditorView,
  from: number,
  to: number,
  activeLines: Set<number>,
  decos: Range<Decoration>[],
): void {
  const tree = syntaxTree(view.state);

  tree.iterate({
    from,
    to,
    enter: (node) => {
      const name = node.name;

      if (/^ATXHeading[1-6]$/.test(name)) {
        const level = Number(name.slice(-1));
        const lineNo = lineNumberAt(view, node.from);
        decos.push(
          Decoration.line({ class: `cm-atlas-h${level}` }).range(view.state.doc.lineAt(node.from).from),
        );

        if (!activeLines.has(lineNo)) {
          const headerMark = findChild(node.node.firstChild, 'HeaderMark');
          if (headerMark) {
            const markEnd = consumeTrailingSpace(view, headerMark.to, node.to);
            decos.push(hideDeco.range(headerMark.from, markEnd));
          }
        }
        return;
      }

      if (name === 'Emphasis' || name === 'StrongEmphasis') {
        const cls = name === 'StrongEmphasis' ? 'cm-atlas-strong' : 'cm-atlas-em';
        const lineNo = lineNumberAt(view, node.from);
        decos.push(Decoration.mark({ class: cls }).range(node.from, node.to));
        if (!activeLines.has(lineNo)) hideMarks(node.node, 'EmphasisMark', decos);
        return;
      }

      if (name === 'Strikethrough') {
        const lineNo = lineNumberAt(view, node.from);
        decos.push(Decoration.mark({ class: 'cm-atlas-strike' }).range(node.from, node.to));
        if (!activeLines.has(lineNo)) hideMarks(node.node, 'StrikethroughMark', decos);
        return;
      }

      if (name === 'InlineCode') {
        const lineNo = lineNumberAt(view, node.from);
        decos.push(Decoration.mark({ class: 'cm-atlas-code' }).range(node.from, node.to));
        if (!activeLines.has(lineNo)) hideMarks(node.node, 'CodeMark', decos);
        return;
      }

      if (name === 'Image') {
        const lineNo = lineNumberAt(view, node.from);
        if (!activeLines.has(lineNo)) {
          const parsed = parseImage(view.state.doc.sliceString(node.from, node.to));
          if (parsed !== null) {
            decos.push(
              Decoration.replace({ widget: new ImageWidget(parsed.url, parsed.alt) }).range(
                node.from,
                node.to,
              ),
            );
          }
        }
        return false;
      }

      if (name === 'Link') {
        decorateLink(view, node.node, activeLines, decos);
        return;
      }

      if (name === 'Blockquote') {
        decorateLines(view, node.from, node.to, 'cm-atlas-quote', decos);
        return;
      }

      if (name === 'QuoteMark') {
        const lineNo = lineNumberAt(view, node.from);
        if (!activeLines.has(lineNo)) {
          const lineEnd = view.state.doc.lineAt(node.from).to;
          decos.push(hideDeco.range(node.from, consumeTrailingSpace(view, node.to, lineEnd)));
        }
        return;
      }

      if (name === 'ListMark') {
        const lineNo = lineNumberAt(view, node.from);
        if (!activeLines.has(lineNo)) {
          const markText = view.state.doc.sliceString(node.from, node.to);
          const isBullet = markText === '-' || markText === '*' || markText === '+';
          const isTask = node.node.parent !== null && hasChild(node.node.parent, 'Task');

          // Task items render a checkbox in place of the marker, so the bullet is
          // hidden entirely (marker + its trailing space) to avoid "• ☑ item".
          if (isTask) {
            const lineEnd = view.state.doc.lineAt(node.from).to;
            decos.push(hideDeco.range(node.from, consumeTrailingSpace(view, node.to, lineEnd)));
          } else if (isBullet) {
            decos.push(Decoration.replace({ widget: new BulletWidget() }).range(node.from, node.to));
          }
        }
        return;
      }

      if (name === 'TaskMarker') {
        const lineNo = lineNumberAt(view, node.from);
        if (!activeLines.has(lineNo)) {
          const checked = taskMarkerChecked(view.state.doc.sliceString(node.from, node.to));
          decos.push(
            Decoration.replace({ widget: new CheckboxWidget(checked, node.from) }).range(node.from, node.to),
          );
        }
        return;
      }

      if (name === 'HorizontalRule') {
        decos.push(Decoration.line({ class: 'cm-atlas-hr' }).range(view.state.doc.lineAt(node.from).from));
        return;
      }

      if (name === 'FencedCode') {
        decorateFenced(view, node.node, activeLines, decos);
        return;
      }

      if (name === 'ListItem') {
        decos.push(
          Decoration.line({ class: 'cm-atlas-listitem' }).range(view.state.doc.lineAt(node.from).from),
        );
        return;
      }
    },
  });
}

/**
 * Standard markdown link `[text](url)`. Off active line: hide `[`, and the
 * `](url)` tail, leaving the link text styled. On active line: leave raw. The
 * collapse is best-effort and relies on the Lezer LinkMark / URL children.
 */
function decorateLink(
  view: EditorView,
  node: SyntaxNode,
  activeLines: Set<number>,
  decos: Range<Decoration>[],
): void {
  const lineNo = lineNumberAt(view, node.from);
  decos.push(Decoration.mark({ class: 'cm-atlas-link' }).range(node.from, node.to));

  if (activeLines.has(lineNo)) return;

  const marks = collectChildren(node, 'LinkMark');
  const url = findChild(node.firstChild, 'URL');

  // marks are: [ "[", "]", "(", ")" ] in document order for [text](url).
  const open = marks[0];
  const closeText = marks[1];

  if (open) decos.push(hideDeco.range(open.from, open.to));

  if (closeText && url) {
    // Hide from the closing "]" through the closing ")" (covers "](url)").
    decos.push(hideDeco.range(closeText.from, node.to));
  }
}

/**
 * Fenced code block. Every line gets the code background. Off the active line(s),
 * the opening ```` ```lang ```` collapses to a language badge (or hides, when no
 * language) and the closing ```` ``` ```` hides, so the block reads as code with a
 * label instead of raw backticks. On an active fence line the markers stay raw.
 */
function decorateFenced(
  view: EditorView,
  node: SyntaxNode,
  activeLines: Set<number>,
  decos: Range<Decoration>[],
): void {
  const doc = view.state.doc;
  decorateLines(view, node.from, node.to, 'cm-atlas-fenced', decos);

  const marks = collectChildren(node, 'CodeMark');
  const openMark = marks[0];
  const closeMark = marks[marks.length - 1];
  const info = findChild(node.firstChild, 'CodeInfo');

  if (openMark) {
    const openLine = doc.lineAt(openMark.from).number;
    if (!activeLines.has(openLine)) {
      const end = info ? info.to : openMark.to;
      const lang = info ? fenceLanguage(doc.sliceString(info.from, info.to)) : null;
      const deco = lang ? Decoration.replace({ widget: new LangBadgeWidget(lang) }) : hideDeco;
      decos.push(deco.range(openMark.from, end));
    }
  }

  if (closeMark && closeMark !== openMark) {
    const closeLine = doc.lineAt(closeMark.from).number;
    if (!activeLines.has(closeLine)) {
      decos.push(hideDeco.range(closeMark.from, closeMark.to));
    }
  }
}

/** Regex pass for wikilinks, which are not in the Lezer markdown grammar. */
function decorateWikilinks(
  view: EditorView,
  from: number,
  to: number,
  activeLines: Set<number>,
  callbacks: LivePreviewCallbacks,
  titles: Record<string, string>,
  decos: Range<Decoration>[],
): void {
  const text = view.state.doc.sliceString(from, to);
  WIKILINK_RE.lastIndex = 0;

  for (let m = WIKILINK_RE.exec(text); m !== null; m = WIKILINK_RE.exec(text)) {
    const inner = m[1];
    if (inner === undefined) continue;

    const start = from + m.index;
    const end = start + m[0].length;
    const lineNo = lineNumberAt(view, start);

    if (activeLines.has(lineNo)) {
      decos.push(Decoration.mark({ class: 'cm-atlas-wikilink-raw' }).range(start, end));
      continue;
    }

    const ref = parseWikilinkInner(inner);
    const display = ref.id !== null ? (titles[ref.id] ?? ref.title) : ref.title;
    decos.push(
      Decoration.replace({ widget: new WikilinkWidget(ref, display, callbacks.onWikilinkClick) }).range(
        start,
        end,
      ),
    );
  }
}

function decorateLines(
  view: EditorView,
  from: number,
  to: number,
  cls: string,
  decos: Range<Decoration>[],
): void {
  const doc = view.state.doc;
  const firstLine = doc.lineAt(from).number;
  const lastLine = doc.lineAt(to).number;

  for (let n = firstLine; n <= lastLine; n += 1) {
    decos.push(Decoration.line({ class: cls }).range(doc.line(n).from));
  }
}

function hideMarks(node: SyntaxNode, markName: string, decos: Range<Decoration>[]): void {
  for (const mark of collectChildren(node, markName)) {
    decos.push(hideDeco.range(mark.from, mark.to));
  }
}

function collectChildren(node: SyntaxNode, name: string): SyntaxNode[] {
  const out: SyntaxNode[] = [];
  for (let child = node.firstChild; child !== null; child = child.nextSibling) {
    if (child.name === name) out.push(child);
  }
  return out;
}

function hasChild(node: SyntaxNode, name: string): boolean {
  for (let child = node.firstChild; child !== null; child = child.nextSibling) {
    if (child.name === name) return true;
  }
  return false;
}

function findChild(start: SyntaxNode | null, name: string): SyntaxNode | null {
  for (let child = start; child !== null; child = child.nextSibling) {
    if (child.name === name) return child;
  }
  return null;
}

/**
 * Extends a marker range to swallow one trailing space, so hiding `### ` removes
 * the gap before the heading text rather than leaving a leading indent. Bounded
 * by `limit` so it never crosses into the content.
 */
function consumeTrailingSpace(view: EditorView, pos: number, limit: number): number {
  if (pos < limit && view.state.doc.sliceString(pos, pos + 1) === ' ') return pos + 1;
  return pos;
}

/**
 * Creates the live-preview extension. Rebuilds the decoration set on every doc
 * change, selection change, and viewport change so reveal-on-active-line tracks
 * the cursor in real time.
 */
export function livePreview(callbacks: LivePreviewCallbacks, options: LivePreviewOptions) {
  const { reveal } = options;
  const titles = options.titles ?? {};

  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;

      constructor(view: EditorView) {
        // The markdown grammar parses incrementally: on first construction the
        // syntax tree may not yet cover the viewport, which would leave the doc
        // rendered as raw markdown until the first interaction. Force the parse
        // up to the viewport so the very first paint is already decorated.
        ensureSyntaxTree(view.state, view.viewport.to, 100);
        this.decorations = buildDecorations(view, callbacks, reveal, titles);
      }

      update(update: ViewUpdate): void {
        // Rebuild on the obvious triggers, and ALSO when the syntax tree changed:
        // the parser dispatches tree-progress transactions that carry none of the
        // doc/selection/viewport flags, and skipping them is what made decorations
        // appear only after the first click.
        if (
          update.docChanged ||
          update.selectionSet ||
          update.viewportChanged ||
          syntaxTree(update.startState) !== syntaxTree(update.state)
        ) {
          this.decorations = buildDecorations(update.view, callbacks, reveal, titles);
        }
      }
    },
    {
      decorations: (plugin) => plugin.decorations,
      provide: (plugin) =>
        EditorView.atomicRanges.of((view) => view.plugin(plugin)?.decorations ?? Decoration.none),
    },
  );
}
