import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { type EditorState, type Extension, type Range, RangeSetBuilder, StateField } from '@codemirror/state';
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import {
  fenceLanguage,
  type InlineToken,
  isBlockActive,
  type ParsedTable,
  parseImage,
  parseTable,
  taskMarkerChecked,
  tokenizeInline,
} from '@/lib/livePreview';
import { parseWikilinkInner, type WikilinkRef } from '@/lib/wikilink';

/**
 * Lezer syntax node, derived from `Tree.resolve` so we do not depend on the
 * transitive `@lezer/common` package being hoisted into node_modules.
 */
type SyntaxNode = ReturnType<ReturnType<typeof syntaxTree>['resolve']>;

/** A document range replaced by a block widget (table, diagram). */
interface BlockRange {
  from: number;
  to: number;
}

/** Context for rendering inline markdown inside block widgets (table cells). */
interface InlineCtx {
  titles: Record<string, string>;
  onWikilinkClick: (ref: WikilinkRef) => void;
}

/**
 * Builds a safe DOM node for one inline token. Text is set via textContent (never
 * innerHTML), so cell content cannot inject markup. Wikilinks resolve their
 * current title and are clickable through the same callback as the editor.
 */
function inlineNode(token: InlineToken, ctx: InlineCtx): Node {
  if (token.type === 'text') return document.createTextNode(token.value);

  if (token.type === 'link') {
    const a = document.createElement('a');
    a.className = 'cm-atlas-link';
    a.textContent = token.value;
    a.href = token.url;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    return a;
  }

  if (token.type === 'wikilink') {
    const ref = parseWikilinkInner(token.value);
    const span = document.createElement('span');
    span.className = 'cm-atlas-wikilink';
    span.textContent = ref.id !== null ? (ctx.titles[ref.id] ?? ref.title) : ref.title;
    span.addEventListener('mousedown', (event) => {
      event.preventDefault();
      ctx.onWikilinkClick(ref);
    });
    return span;
  }

  const cls = {
    code: 'cm-atlas-code',
    strong: 'cm-atlas-strong',
    em: 'cm-atlas-em',
    strike: 'cm-atlas-strike',
  }[token.type];
  const span = document.createElement('span');
  span.className = cls;
  span.textContent = token.value;
  return span;
}

/** Renders a cell's inline markdown into `parent` as formatted DOM nodes. */
function appendInline(parent: HTMLElement, text: string, ctx: InlineCtx): void {
  for (const token of tokenizeInline(text)) parent.appendChild(inlineNode(token, ctx));
}

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
 * The active-line rule (a line is active when a selection range touches it) is
 * applied directly from the selection via `activeLinesFromSelection`; the pure
 * `computeActiveLines` helper in `lib/livePreview` encodes the same rule and stays
 * the unit-testable reference for it.
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

/**
 * Block widget that renders a GFM table as an HTML `<table>` off the active block.
 * Clicking it (when editable) drops the caret at the table's start, which reveals
 * the raw markdown so it can be edited. Cell content renders inline markdown
 * (bold, italic, code, strikethrough, links, wikilinks).
 */
class TableWidget extends WidgetType {
  constructor(
    private readonly table: ParsedTable,
    private readonly from: number,
    private readonly ctx: InlineCtx,
  ) {
    super();
  }

  eq(other: TableWidget): boolean {
    return other.from === this.from && other.key === this.key;
  }

  // Includes the resolved titles so cells with wikilinks re-render on rename.
  private get key(): string {
    return JSON.stringify(this.table) + JSON.stringify(this.ctx.titles);
  }

  toDOM(view: EditorView): HTMLElement {
    const wrap = document.createElement('div');
    wrap.className = 'cm-atlas-table-wrap';

    const table = document.createElement('table');
    table.className = 'cm-atlas-table';

    const cols = this.table.headers.length;
    const align = (cell: HTMLTableCellElement, index: number): void => {
      const a = this.table.aligns[index];
      if (a) cell.style.textAlign = a;
    };

    const thead = document.createElement('thead');
    const headRow = document.createElement('tr');
    this.table.headers.forEach((text, i) => {
      const th = document.createElement('th');
      appendInline(th, text, this.ctx);
      align(th, i);
      headRow.appendChild(th);
    });
    thead.appendChild(headRow);
    table.appendChild(thead);

    const tbody = document.createElement('tbody');
    for (const row of this.table.rows) {
      const tr = document.createElement('tr');
      for (let i = 0; i < cols; i += 1) {
        const td = document.createElement('td');
        appendInline(td, row[i] ?? '', this.ctx);
        align(td, i);
        tr.appendChild(td);
      }
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    wrap.appendChild(table);

    wrap.addEventListener('mousedown', (event) => {
      if (view.state.readOnly) return;
      event.preventDefault();
      view.dispatch({ selection: { anchor: this.from }, scrollIntoView: true });
      view.focus();
    });

    return wrap;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

// Mermaid is heavy, so it is imported lazily on first use and cached. The render
// runs with `securityLevel: 'strict'` so the produced SVG is sanitised.
type MermaidApi = {
  initialize: (config: Record<string, unknown>) => void;
  render: (id: string, code: string) => Promise<{ svg: string }>;
};
let mermaidPromise: Promise<MermaidApi> | null = null;
let mermaidSeq = 0;

function loadMermaid(): Promise<MermaidApi> {
  if (mermaidPromise === null) {
    mermaidPromise = import('mermaid').then((m) => m.default as unknown as MermaidApi);
  }
  return mermaidPromise;
}

/** Maps the app theme (`data-theme` on <html>) to a built-in mermaid theme. */
function currentMermaidTheme(): 'dark' | 'default' {
  return document.documentElement.dataset.theme === 'light' ? 'default' : 'dark';
}

async function renderMermaid(container: HTMLElement, code: string): Promise<void> {
  try {
    const mermaid = await loadMermaid();
    mermaidSeq += 1;
    // Theme is set per render so the diagram tracks the app's dark/light theme.
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: currentMermaidTheme() });
    const { svg } = await mermaid.render(`atlas-mermaid-${mermaidSeq}`, code);
    container.innerHTML = svg;
    container.classList.remove('cm-atlas-mermaid-error');
  } catch {
    container.textContent = code;
    container.classList.add('cm-atlas-mermaid-error');
  }
}

// Per-diagram observers that re-render when the app theme flips, keyed by the
// widget DOM so they can be disconnected when the widget is destroyed.
const mermaidThemeObservers = new WeakMap<HTMLElement, MutationObserver>();

/**
 * Block widget that renders a ```mermaid code block as a diagram. The diagram is
 * rendered asynchronously (mermaid is lazy-loaded) with the current app theme and
 * re-rendered when the theme changes; on a parse error the raw code is shown
 * instead. Clicking (when editable) reveals the source for editing.
 */
class MermaidWidget extends WidgetType {
  constructor(
    private readonly code: string,
    private readonly from: number,
  ) {
    super();
  }

  eq(other: MermaidWidget): boolean {
    return other.code === this.code && other.from === this.from;
  }

  toDOM(view: EditorView): HTMLElement {
    const wrap = document.createElement('div');
    wrap.className = 'cm-atlas-mermaid';

    wrap.addEventListener('mousedown', (event) => {
      if (view.state.readOnly) return;
      event.preventDefault();
      view.dispatch({ selection: { anchor: this.from }, scrollIntoView: true });
      view.focus();
    });

    void renderMermaid(wrap, this.code);

    if (typeof MutationObserver !== 'undefined') {
      const observer = new MutationObserver(() => void renderMermaid(wrap, this.code));
      observer.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ['data-theme'],
      });
      mermaidThemeObservers.set(wrap, observer);
    }

    return wrap;
  }

  destroy(dom: HTMLElement): void {
    mermaidThemeObservers.get(dom)?.disconnect();
    mermaidThemeObservers.delete(dom);
  }

  ignoreEvent(): boolean {
    return false;
  }
}

const hideDeco = Decoration.replace({});

/**
 * The set of "active" (revealed) line numbers for the current selection: every
 * line any selection range touches, matching `computeActiveLines`' intersection
 * rule. Derived directly from the selection via `lineAt` — O(selection ranges),
 * not O(document lines) — so it stays cheap on every keystroke and caret move in
 * large documents. Returns an empty set when reveal is off (preview / read-only).
 */
export function activeLinesFromSelection(state: EditorState, reveal: boolean): Set<number> {
  const active = new Set<number>();
  if (!reveal) return active;

  const doc = state.doc;
  for (const range of state.selection.ranges) {
    const first = doc.lineAt(Math.min(range.from, range.to)).number;
    const last = doc.lineAt(Math.max(range.from, range.to)).number;
    for (let n = first; n <= last; n += 1) active.add(n);
  }

  return active;
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
  const activeLines = activeLinesFromSelection(view.state, reveal);
  const decos: Range<Decoration>[] = [];

  // Ranges replaced by a block widget (tables, diagrams). The wikilink pass must
  // skip these: a replace decoration inside an already-replaced block would
  // overlap and break the RangeSet. Collect them in a full first pass so every
  // block range is known before any wikilink is added.
  const blockRanges: BlockRange[] = [];

  for (const { from, to } of view.visibleRanges) {
    decorateSyntaxTree(view, from, to, activeLines, decos, blockRanges);
  }
  for (const { from, to } of view.visibleRanges) {
    decorateWikilinks(view, from, to, activeLines, callbacks, titles, decos, blockRanges);
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
  blockRanges: BlockRange[],
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

      if (name === 'Table') {
        // The rendered table is a BLOCK decoration, which CodeMirror only allows
        // from a StateField (see blockDecorationsField). Here the ViewPlugin just
        // records the range so its inline/wikilink passes skip inside it; the
        // actual widget is produced by the field with the same active-block rule.
        const doc = view.state.doc;
        const firstLine = doc.lineAt(node.from).number;
        const lastLine = doc.lineAt(node.to).number;
        if (!isBlockActive(firstLine, lastLine, activeLines)) {
          blockRanges.push({ from: node.from, to: node.to });
        }
        return false;
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
        // A ```mermaid block renders as a diagram, which is a BLOCK decoration
        // owned by the StateField. Here the ViewPlugin just records the range so
        // its passes skip inside it; everything else is a normal fenced block.
        if (fencedLanguage(view.state, node.node) === 'mermaid') {
          const doc = view.state.doc;
          const firstLine = doc.lineAt(node.from).number;
          const lastLine = doc.lineAt(node.to).number;
          if (!isBlockActive(firstLine, lastLine, activeLines)) {
            blockRanges.push({ from: node.from, to: node.to });
          }
          return false;
        }
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
  decorateLines(view, node.from, node.to, 'cm-atlas-fenced', decos, {
    first: 'cm-atlas-fenced-first',
    last: 'cm-atlas-fenced-last',
  });

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
  blockRanges: BlockRange[],
): void {
  const text = view.state.doc.sliceString(from, to);
  WIKILINK_RE.lastIndex = 0;

  for (let m = WIKILINK_RE.exec(text); m !== null; m = WIKILINK_RE.exec(text)) {
    const inner = m[1];
    if (inner === undefined) continue;

    const start = from + m.index;
    const end = start + m[0].length;

    // Skip wikilinks inside a block-replaced range (e.g. a rendered table cell):
    // a replace inside an already-replaced block would overlap and throw.
    if (isInsideBlock(start, blockRanges)) continue;

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
  edge?: { first: string; last: string },
): void {
  const doc = view.state.doc;
  const firstLine = doc.lineAt(from).number;
  const lastLine = doc.lineAt(to).number;

  for (let n = firstLine; n <= lastLine; n += 1) {
    let lineCls = cls;
    if (edge !== undefined) {
      if (n === firstLine) lineCls += ` ${edge.first}`;
      if (n === lastLine) lineCls += ` ${edge.last}`;
    }
    decos.push(Decoration.line({ class: lineCls }).range(doc.line(n).from));
  }
}

function isInsideBlock(pos: number, blockRanges: BlockRange[]): boolean {
  return blockRanges.some((b) => pos >= b.from && pos < b.to);
}

/** The language label of a FencedCode node from its CodeInfo child, or null. */
function fencedLanguage(state: EditorState, node: SyntaxNode): string | null {
  const info = findChild(node.firstChild, 'CodeInfo');
  return info ? fenceLanguage(state.doc.sliceString(info.from, info.to)) : null;
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
 * Block-level nodes whose subtree is pure inline content: a table or fenced-code
 * block can never appear inside one. Skipping their descent keeps the whole-document
 * block walk from visiting every inline node (emphasis, links, code, text) on each
 * keystroke and caret move. Container blocks (lists, blockquotes) are intentionally
 * absent so a table nested in them is still discovered.
 */
const INLINE_ONLY_BLOCKS = new Set([
  'Paragraph',
  'ATXHeading1',
  'ATXHeading2',
  'ATXHeading3',
  'ATXHeading4',
  'ATXHeading5',
  'ATXHeading6',
  'SetextHeading1',
  'SetextHeading2',
]);

/**
 * Builds the BLOCK decorations (rendered tables and mermaid diagrams) for the
 * whole document. Block widgets and decorations that span line breaks may only be
 * provided through a StateField, never a ViewPlugin, so these live apart from the
 * inline pass.
 *
 * A block is rendered as a widget unless the selection touches it, in which case
 * it is left as raw markdown for editing (reveal-on-active-block).
 *
 * Exported for unit testing the block-discovery walk without a DOM.
 */
export function buildBlockDecorations(state: EditorState, reveal: boolean, ctx: InlineCtx): DecorationSet {
  const tree = syntaxTree(state);
  const doc = state.doc;
  const activeLines = activeLinesFromSelection(state, reveal);
  const decos: Range<Decoration>[] = [];

  const blockReplace = (node: SyntaxNode, widget: WidgetType): void => {
    decos.push(Decoration.replace({ widget, block: true }).range(node.from, node.to));
  };

  tree.iterate({
    enter: (node) => {
      if (INLINE_ONLY_BLOCKS.has(node.name)) return false;

      if (node.name === 'Table') {
        const firstLine = doc.lineAt(node.from).number;
        const lastLine = doc.lineAt(node.to).number;
        if (!isBlockActive(firstLine, lastLine, activeLines)) {
          const parsed = parseTable(doc.sliceString(node.from, node.to));
          if (parsed !== null) blockReplace(node.node, new TableWidget(parsed, node.from, ctx));
        }
        return false;
      }

      if (node.name === 'FencedCode') {
        if (fencedLanguage(state, node.node) === 'mermaid') {
          const firstLine = doc.lineAt(node.from).number;
          const lastLine = doc.lineAt(node.to).number;
          if (!isBlockActive(firstLine, lastLine, activeLines)) {
            const codeText = findChild(node.node.firstChild, 'CodeText');
            const code = codeText ? doc.sliceString(codeText.from, codeText.to) : '';
            blockReplace(node.node, new MermaidWidget(code, node.from));
          }
        }
        return false;
      }

      return undefined;
    },
  });

  decos.sort((a, b) => a.from - b.from || a.value.startSide - b.value.startSide);

  const builder = new RangeSetBuilder<Decoration>();
  for (const deco of decos) builder.add(deco.from, deco.to, deco.value);
  return builder.finish();
}

/**
 * StateField that provides the block decorations. Recomputed on every doc or
 * selection change so a table re-renders (or reveals raw) as the cursor moves.
 */
function blockDecorationsField(reveal: boolean, ctx: InlineCtx): StateField<DecorationSet> {
  return StateField.define<DecorationSet>({
    create(state) {
      ensureSyntaxTree(state, state.doc.length, 100);
      return buildBlockDecorations(state, reveal, ctx);
    },
    update(value, tr) {
      if (tr.docChanged || tr.selection !== undefined || syntaxTree(tr.startState) !== syntaxTree(tr.state)) {
        return buildBlockDecorations(tr.state, reveal, ctx);
      }
      return value.map(tr.changes);
    },
    provide: (field) => EditorView.decorations.from(field),
  });
}

/**
 * Creates the live-preview extension. The inline / per-line decorations come from
 * a ViewPlugin (rebuilt on doc, selection, viewport and syntax-tree changes); the
 * block decorations (tables) come from a StateField, since CodeMirror forbids a
 * ViewPlugin from emitting block or line-break-spanning decorations.
 */
export function livePreview(callbacks: LivePreviewCallbacks, options: LivePreviewOptions): Extension {
  const { reveal } = options;
  const titles = options.titles ?? {};

  const inline = ViewPlugin.fromClass(
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

  const ctx: InlineCtx = { titles, onWikilinkClick: callbacks.onWikilinkClick };
  return [inline, blockDecorationsField(reveal, ctx)];
}
