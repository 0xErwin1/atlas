import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import {
  type EditorSelection,
  type EditorState,
  type Extension,
  type Range,
  RangeSetBuilder,
  StateField,
} from '@codemirror/state';
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import { BoundedCache } from '@/lib/boundedCache';
import {
  buildDocRangeCache,
  type DocRangeCache,
  fenceLanguage,
  findMathRanges,
  findWikilinkRanges,
  type InlineToken,
  isBlockActive,
  type MathRange,
  overlappingRangeBounds,
  type ParsedTable,
  type PositionRange,
  paragraphSoftBreaks,
  parseImage,
  parseTable,
  rangeContaining,
  shouldRefreshDocRangeCache,
  taskMarkerChecked,
  tokenizeInline,
  type WikilinkRange,
} from '@/lib/livePreview';
import { safeUrl, sanitizeMarkdownHtmlFragment } from '@/lib/sanitize';
import { formatWikilink, parseWikilinkInner, type WikilinkRef, wikilinkDisplay } from '@/lib/wikilink';
import { documentStyleNonce } from './cspNonce';

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
export function inlineNode(token: InlineToken, ctx: InlineCtx): Node {
  if (token.type === 'text') return document.createTextNode(token.value);

  if (token.type === 'math') {
    const span = document.createElement('span');
    paintMath(span, token.value, false, 'cm-atlas-math-inline');
    return span;
  }

  if (token.type === 'link') {
    // A link with a disallowed scheme (javascript:, data:, ...) is rendered as
    // plain text: emitting a live anchor would be a stored DOM XSS sink.
    const href = safeUrl(token.url);
    if (href === null) return document.createTextNode(token.value);

    const a = document.createElement('a');
    a.className = 'cm-atlas-link';
    appendInline(a, token.value, ctx);
    a.href = href;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    return a;
  }

  if (token.type === 'wikilink') {
    const ref = parseWikilinkInner(token.value);
    const span = document.createElement('span');
    span.className = 'cm-atlas-wikilink';
    span.textContent = wikilinkDisplay(ref, ctx.titles);
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
  /**
   * Optional hook to translate a rendered image's source before it is applied.
   * Hosts whose images live behind the Atlas API supply one; see `ImageWidget`.
   */
  resolveImageSrc?: (url: string) => Promise<string | null>;
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
    return formatWikilink(other.ref) === formatWikilink(this.ref) && other.display === this.display;
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

class LinkWidget extends WidgetType {
  constructor(
    private readonly text: string,
    private readonly url: string,
    private readonly ctx: InlineCtx,
  ) {
    super();
  }

  eq(other: LinkWidget): boolean {
    return other.text === this.text && other.url === this.url && other.ctx.titles === this.ctx.titles;
  }

  toDOM(): HTMLElement {
    const href = safeUrl(this.url);
    if (href === null) {
      const span = document.createElement('span');
      span.className = 'cm-atlas-link';
      span.textContent = this.text;
      return span;
    }

    const a = document.createElement('a');
    a.className = 'cm-atlas-link';
    appendInline(a, this.text, this.ctx);
    a.href = href;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    return a;
  }

  ignoreEvent(): boolean {
    return true;
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
 *
 * `resolveSrc` lets a host substitute the source before it reaches the `<img>`,
 * which is how images hosted by the Atlas API are loaded: the webview cannot
 * request them itself (see `useApiImageSrc`). Without it the URL is used verbatim.
 */
export class ImageWidget extends WidgetType {
  constructor(
    private readonly url: string,
    private readonly alt: string,
    private readonly resolveSrc?: (url: string) => Promise<string | null>,
  ) {
    super();
  }

  eq(other: ImageWidget): boolean {
    return other.url === this.url && other.alt === this.alt && other.resolveSrc === this.resolveSrc;
  }

  toDOM(view: EditorView): HTMLElement {
    const src = safeUrl(this.url);

    // A disallowed scheme (javascript:, data:, ...) is never set as `src`; the
    // image collapses to its alt text instead of becoming an XSS sink.
    if (src === null) {
      const span = document.createElement('span');
      span.className = 'cm-atlas-img';
      span.textContent = this.alt;
      return span;
    }

    const img = document.createElement('img');
    img.className = 'cm-atlas-img';
    img.alt = this.alt;

    // The image loads asynchronously, so its height is unknown when CodeMirror
    // first measures the widget. Without a re-measure on load the height map keeps
    // the collapsed placeholder height and everything below the image is mapped to
    // the wrong vertical position. Requesting a measure once the natural size is
    // known reconciles the height map with the rendered image.
    const remeasure = (): void => {
      if (img.isConnected) view.requestMeasure();
    };
    img.addEventListener('load', remeasure, { once: true });
    img.addEventListener('error', remeasure, { once: true });

    if (this.resolveSrc === undefined) {
      img.src = src;
      return img;
    }

    // An unresolvable source leaves `src` unset on purpose: the browser then shows
    // the alt text rather than a broken-image icon.
    void this.resolveSrc(src).then((resolved) => {
      if (resolved !== null) img.src = resolved;
      remeasure();
    });

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
  // Includes the resolved titles so cells with wikilinks re-render on rename.
  private readonly key: string;

  constructor(
    private readonly table: ParsedTable,
    private readonly from: number,
    private readonly ctx: InlineCtx,
  ) {
    super();
    this.key = JSON.stringify(table) + JSON.stringify(ctx.titles);
  }

  eq(other: TableWidget): boolean {
    return other.from === this.from && other.key === this.key;
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
      if (event.target instanceof Element && event.target.closest('a')) return;
      event.preventDefault();
      view.dispatch({ selection: { anchor: this.from }, scrollIntoView: true });
      view.focus();
    });

    return wrap;
  }

  ignoreEvent(): boolean {
    return true;
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

export function attachMermaidSvg(container: HTMLElement, svg: string): void {
  const template = document.createElement('template');
  template.innerHTML = svg;

  const nonce = documentStyleNonce();
  if (nonce !== '') {
    for (const style of template.content.querySelectorAll('style')) {
      style.setAttribute('nonce', nonce);
    }
  }

  container.replaceChildren(template.content);
}

const MERMAID_CACHE_CAP = 64;

/**
 * Rendered SVG per diagram source and theme. CodeMirror drops a widget's DOM
 * when its block leaves the viewport margin and asks for it again on re-entry,
 * so without this every scroll past a diagram would run mermaid again.
 */
const mermaidSvgCache = new BoundedCache<string | null>(MERMAID_CACHE_CAP);

/**
 * Last measured wrapper height per diagram source, reserved before the SVG
 * lands so the block does not jump from 0px once the render resolves. Keyed
 * by source only: the palette does not change a diagram's size.
 */
const mermaidHeightCache = new BoundedCache<number>(MERMAID_CACHE_CAP);

/** In-flight renders per cache key, so widgets sharing a source share one render. */
const mermaidInFlight = new Map<string, Promise<string | null>>();

interface LiveMermaid {
  code: string;
  view: EditorView;
}

/** Mounted diagram wrappers, re-rendered together when the app theme flips. */
const liveMermaidWrappers = new Map<HTMLElement, LiveMermaid>();
let mermaidThemeObserver: MutationObserver | null = null;

function mermaidCacheKey(code: string): string {
  return `${currentMermaidTheme()}\u0000${code}`;
}

/**
 * Renders `code` with the current theme, resolving to the SVG or null on a
 * parse error. The result is cached either way, so a broken diagram is not
 * re-parsed on every viewport entry; concurrent callers share one render.
 */
function renderMermaidSvg(code: string): Promise<string | null> {
  const key = mermaidCacheKey(code);
  const pending = mermaidInFlight.get(key);
  if (pending !== undefined) return pending;

  const run = (async (): Promise<string | null> => {
    try {
      const mermaid = await loadMermaid();
      mermaidSeq += 1;
      // Theme is set per render so the diagram tracks the app's dark/light theme.
      mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: currentMermaidTheme() });
      const { svg } = await mermaid.render(`atlas-mermaid-${mermaidSeq}`, code);
      mermaidSvgCache.set(key, svg);
      return svg;
    } catch {
      mermaidSvgCache.set(key, null);
      return null;
    } finally {
      mermaidInFlight.delete(key);
    }
  })();

  mermaidInFlight.set(key, run);
  return run;
}

function paintMermaid(wrap: HTMLElement, code: string, svg: string | null): void {
  wrap.style.minHeight = '';

  if (svg === null) {
    wrap.textContent = code;
    wrap.classList.add('cm-atlas-mermaid-error');
    return;
  }

  attachMermaidSvg(wrap, svg);
  wrap.classList.remove('cm-atlas-mermaid-error');
}

/**
 * Records the painted wrapper's height for later reservation and lets the view
 * reconcile its height map with the diagram that just landed.
 */
function measureMermaid(wrap: HTMLElement, code: string, view: EditorView): void {
  view.requestMeasure({
    read: () => wrap.getBoundingClientRect().height,
    write: (height) => {
      if (height > 0) mermaidHeightCache.set(code, height);
    },
  });
}

/**
 * Paints the diagram into `wrap`: synchronously from the cache when the source
 * was rendered before under the current theme, otherwise asynchronously after a
 * render, with the last known height reserved in the meantime.
 */
function renderMermaidInto(wrap: HTMLElement, code: string, view: EditorView): void {
  const cached = mermaidSvgCache.get(mermaidCacheKey(code));
  if (cached !== undefined) {
    paintMermaid(wrap, code, cached);
    if (mermaidHeightCache.get(code) === undefined) measureMermaid(wrap, code, view);
    return;
  }

  const height = mermaidHeightCache.get(code);
  if (height !== undefined) wrap.style.minHeight = `${height}px`;

  void renderMermaidSvg(code).then((svg) => {
    if (!liveMermaidWrappers.has(wrap)) return;
    paintMermaid(wrap, code, svg);
    measureMermaid(wrap, code, view);
  });
}

/**
 * Starts the single observer that re-renders every mounted diagram when the
 * app theme flips. Cached SVG embeds the old palette, so it is dropped first.
 */
function watchMermaidTheme(): void {
  if (mermaidThemeObserver !== null || typeof MutationObserver === 'undefined') return;

  mermaidThemeObserver = new MutationObserver(() => {
    mermaidSvgCache.clear();
    for (const [wrap, live] of liveMermaidWrappers) renderMermaidInto(wrap, live.code, live.view);
  });
  mermaidThemeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });
}

interface RenderedMath {
  html: string;
  invalid: boolean;
}

function mathBody(doc: EditorState['doc'], range: MathRange): string {
  return doc.sliceString(range.bodyFrom, range.bodyTo).trim();
}

// KaTeX is heavy, so it is imported lazily the first time a document renders math
// and cached. The resolved module is kept alongside the promise so every later
// widget still paints synchronously, exactly as a static import did.
type KatexApi = {
  renderToString: (formula: string, options: Record<string, unknown>) => string;
};
let katexApi: KatexApi | null = null;
let katexPromise: Promise<KatexApi> | null = null;

function loadKatex(): Promise<KatexApi> {
  if (katexPromise === null) {
    katexPromise = import('katex').then((m) => {
      katexApi = m.default as unknown as KatexApi;
      return katexApi;
    });
  }
  return katexPromise;
}

const MATH_CACHE_CAP = 256;

/**
 * Rendered KaTeX output per formula and mode. A math widget is re-created every
 * time its line re-enters the viewport, and KaTeX parsing is the expensive part,
 * so the markup is rendered once per distinct formula.
 */
const mathRenderCache = new BoundedCache<RenderedMath>(MATH_CACHE_CAP);

/**
 * Renders `formula` to HTML-only markup (no MathML twin: it is visually hidden
 * yet doubles the DOM every formula costs at layout time), memoised per formula
 * and display mode.
 */
function renderMath(katex: KatexApi, formula: string, displayMode: boolean): RenderedMath {
  const key = `${displayMode ? 'display' : 'inline'}\u0000${formula}`;
  const cached = mathRenderCache.get(key);
  if (cached !== undefined) return cached;

  let rendered: RenderedMath;
  try {
    const html = katex.renderToString(formula, {
      displayMode,
      output: 'html',
      throwOnError: false,
      trust: false,
    });
    rendered = { html, invalid: /katex-error|merror/.test(html) };
  } catch {
    rendered = { html: '', invalid: true };
  }

  mathRenderCache.set(key, rendered);
  return rendered;
}

function appendMathFallback(parent: HTMLElement, formula: string): void {
  const label = document.createElement('span');
  label.className = 'cm-atlas-math-error-label';
  label.textContent = 'Invalid math';
  parent.appendChild(label);

  const source = document.createElement('code');
  source.textContent = formula;
  parent.appendChild(source);
}

function showMathFallback(target: HTMLElement, formula: string, baseClass: string): void {
  target.className = `${baseClass} cm-atlas-math-error`;
  target.setAttribute('role', 'note');
  appendMathFallback(target, formula);
}

/**
 * Paints `formula` into `target` immediately when KaTeX is already loaded, and
 * otherwise fills the widget in once the lazy import resolves. The widget stays
 * empty while loading rather than showing its source, so the first math document
 * of a session never flashes raw TeX.
 */
function paintMath(target: HTMLElement, formula: string, displayMode: boolean, baseClass: string): void {
  if (katexApi !== null) {
    applyMath(target, katexApi, formula, displayMode, baseClass);
    return;
  }

  target.className = baseClass;

  void loadKatex()
    .then((katex) => applyMath(target, katex, formula, displayMode, baseClass))
    .catch(() => showMathFallback(target, formula, baseClass));
}

function applyMath(
  target: HTMLElement,
  katex: KatexApi,
  formula: string,
  displayMode: boolean,
  baseClass: string,
): void {
  const rendered = renderMath(katex, formula, displayMode);

  if (rendered.invalid) {
    showMathFallback(target, formula, baseClass);
    return;
  }

  target.className = baseClass;
  target.innerHTML = rendered.html;
}

class MathInlineWidget extends WidgetType {
  constructor(private readonly formula: string) {
    super();
  }

  eq(other: MathInlineWidget): boolean {
    return other.formula === this.formula;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    paintMath(span, this.formula, false, 'cm-atlas-math-inline');
    return span;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

class MathBlockWidget extends WidgetType {
  constructor(
    private readonly formula: string,
    private readonly from: number,
  ) {
    super();
  }

  eq(other: MathBlockWidget): boolean {
    return other.formula === this.formula && other.from === this.from;
  }

  toDOM(view: EditorView): HTMLElement {
    const wrap = document.createElement('div');
    paintMath(wrap, this.formula, true, 'cm-atlas-math-block');

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

class HtmlBlockWidget extends WidgetType {
  constructor(
    private readonly html: string,
    private readonly from: number,
  ) {
    super();
  }

  eq(other: HtmlBlockWidget): boolean {
    return other.html === this.html && other.from === this.from;
  }

  toDOM(view: EditorView): HTMLElement {
    const wrap = document.createElement('div');
    wrap.className = 'cm-atlas-html-block';
    wrap.appendChild(sanitizeMarkdownHtmlFragment(this.html));

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

/**
 * Block widget that renders a ```mermaid code block as a diagram. The diagram is
 * rendered asynchronously (mermaid is lazy-loaded) with the current app theme and
 * re-rendered when the theme changes; on a parse error the raw code is shown
 * instead. Clicking (when editable) reveals the source for editing.
 *
 * Exported for unit testing its identity and height reservation.
 */
export class MermaidWidget extends WidgetType {
  constructor(
    private readonly code: string,
    private readonly from: number,
  ) {
    super();
  }

  eq(other: MermaidWidget): boolean {
    return other.code === this.code && other.from === this.from;
  }

  get estimatedHeight(): number {
    return mermaidHeightCache.get(this.code) ?? -1;
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

    liveMermaidWrappers.set(wrap, { code: this.code, view });
    watchMermaidTheme();
    renderMermaidInto(wrap, this.code, view);

    return wrap;
  }

  destroy(dom: HTMLElement): void {
    liveMermaidWrappers.delete(dom);
  }

  ignoreEvent(): boolean {
    return false;
  }
}

const hideDeco = Decoration.replace({});

/**
 * Widget that stands in for a paragraph's soft line break, rendering the single
 * space CommonMark says it is. Replacing the newline itself is what makes
 * CodeMirror join the two source lines into one visual line, so a hard-wrapped
 * paragraph reflows to the container width instead of keeping the source's wraps.
 */
class SoftBreakWidget extends WidgetType {
  eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-atlas-softbreak';
    span.textContent = ' ';
    return span;
  }

  ignoreEvent(): boolean {
    return true;
  }
}

const softBreakDeco = Decoration.replace({ widget: new SoftBreakWidget() });

/**
 * Milliseconds of synchronous parse work allowed to bring the syntax tree up to
 * the viewport before decorations are built. The markdown grammar parses
 * incrementally and CodeMirror's initial parse only covers the first ~3 KB of the
 * document (`Work.InitViewport`), so a larger document's viewport would otherwise
 * be decorated against a tree that stops short of what is on screen. A viewport is
 * bounded, so this budget is ample and effectively never exceeded in practice.
 */
const VIEWPORT_PARSE_BUDGET_MS = 100;

/**
 * The syntax tree that spans at least the current viewport, forcing incremental
 * parse work up to `viewport.to` when the background parser has not reached it
 * yet. `ensureSyntaxTree` advances the parse and RETURNS the extended tree, but it
 * does not update the `Language` state field, so `syntaxTree(view.state)` would
 * still report the stale, short init tree — decorations must read the returned
 * tree instead. Falls back to the state-field tree if the budget is exceeded (the
 * background parser then fills in the rest and triggers a rebuild).
 */
function viewportSyntaxTree(view: EditorView): ReturnType<typeof syntaxTree> {
  return ensureSyntaxTree(view.state, view.viewport.to, VIEWPORT_PARSE_BUDGET_MS) ?? syntaxTree(view.state);
}

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
/**
 * The rendering decoration set plus the subset that should trap the caret.
 *
 * `decorations` drives the view; `atomic` feeds `EditorView.atomicRanges`. Only
 * ranges that HIDE or REPLACE source (hidden markdown marks, widget-replaced
 * constructs) belong in `atomic`, so the caret skips them and a delete removes
 * the whole construct. Visible mark decorations (inline code, emphasis, links)
 * must stay editable and are therefore excluded.
 */
interface BuiltDecorations {
  decorations: DecorationSet;
  atomic: DecorationSet;
}

/**
 * True for replace/widget decorations, false for the styling marks and line
 * decorations. Every mark/line decoration in this file is built with a `class`;
 * `Decoration.replace(...)` (hidden marks and widgets) never carries one, so the
 * absence of `class` cleanly identifies the ranges that should be atomic.
 */
export function isReplaceDeco(deco: Decoration): boolean {
  return (deco.spec as { class?: unknown }).class === undefined;
}

export function buildDecorations(
  view: EditorView,
  callbacks: LivePreviewCallbacks,
  reveal: boolean,
  titles: Record<string, string>,
  /**
   * Optional full-document math/wikilink scan. When provided (ViewPlugin cache),
   * selection/viewport rebuilds skip `doc.toString()` and range re-discovery.
   */
  rangeCache?: DocRangeCache,
): BuiltDecorations {
  const tree = viewportSyntaxTree(view);
  const activeLines = activeLinesFromSelection(view.state, reveal);
  const decos: Range<Decoration>[] = [];

  // Ranges replaced by a block widget (tables, diagrams). The wikilink pass must
  // skip these: a replace decoration inside an already-replaced block would
  // overlap and break the RangeSet. Collect them in a full first pass so every
  // block range is known before any wikilink is added. Only blocks touching a
  // visible range matter, since every later pass is scoped to those ranges.
  const blockRanges: BlockRange[] = [];
  const docText = rangeCache?.docText ?? view.state.doc.toString();
  const mathRanges = rangeCache?.mathRanges ?? findMathRanges(docText);

  for (const { from, to } of view.visibleRanges) {
    const { start, end } = overlappingRangeBounds(mathRanges, from, to);
    for (let i = start; i < end; i += 1) {
      const range = mathRanges[i];
      if (range === undefined || range.kind !== 'block') continue;
      const firstLine = view.state.doc.lineAt(range.from).number;
      const lastLine = view.state.doc.lineAt(range.to).number;
      if (!isBlockActive(firstLine, lastLine, activeLines))
        blockRanges.push({ from: range.from, to: range.to });
    }

    tree.iterate({
      from,
      to,
      enter: (node) => {
        if (node.name !== 'HTMLBlock') return undefined;
        blockRanges.push({ from: node.from, to: node.to });
        return false;
      },
    });
  }

  // Cached wikilink ranges omit block exclusions; decorateWikilinks filters via
  // isInsideBlock. Without a cache, exclude block ranges during the scan.
  const wikilinkRanges = rangeCache?.wikilinkRanges ?? findWikilinkRanges(docText, blockRanges);

  for (const { from, to } of view.visibleRanges) {
    decorateSyntaxTree(
      view,
      tree,
      from,
      to,
      activeLines,
      callbacks,
      titles,
      decos,
      blockRanges,
      wikilinkRanges,
    );
  }
  for (const { from, to } of view.visibleRanges) {
    decorateInlineMath(view, from, to, activeLines, decos, blockRanges, mathRanges);
  }
  for (const { from, to } of view.visibleRanges) {
    decorateWikilinks(view, from, to, activeLines, callbacks, titles, decos, blockRanges, wikilinkRanges);
  }

  decos.sort((a, b) => a.from - b.from || a.value.startSide - b.value.startSide);

  const builder = new RangeSetBuilder<Decoration>();
  const atomicBuilder = new RangeSetBuilder<Decoration>();
  for (const deco of decos) {
    builder.add(deco.from, deco.to, deco.value);
    if (isReplaceDeco(deco.value)) atomicBuilder.add(deco.from, deco.to, deco.value);
  }

  return { decorations: builder.finish(), atomic: atomicBuilder.finish() };
}

/**
 * Walks the Lezer markdown tree over [from, to] and pushes decorations for every
 * supported construct. The reveal-on-active-line rule is applied per construct:
 * markers on an active line are left raw (only the content styling is applied),
 * markers elsewhere are collapsed with a replace decoration.
 */
function decorateSyntaxTree(
  view: EditorView,
  tree: ReturnType<typeof syntaxTree>,
  from: number,
  to: number,
  activeLines: Set<number>,
  callbacks: LivePreviewCallbacks,
  titles: Record<string, string>,
  decos: Range<Decoration>[],
  blockRanges: BlockRange[],
  wikilinkRanges: WikilinkRange[],
): void {
  tree.iterate({
    from,
    to,
    enter: (node) => {
      const name = node.name;

      if (isInsideRange(node.from, node.to, wikilinkRanges)) return false;

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
              Decoration.replace({
                widget: new ImageWidget(parsed.url, parsed.alt, callbacks.resolveImageSrc),
              }).range(node.from, node.to),
            );
          }
        }
        return false;
      }

      if (name === 'Link') {
        decorateLink(view, node.node, activeLines, callbacks, titles, decos);
        return false;
      }

      if (name === 'Autolink') {
        decorateAutolink(view, node.node, activeLines, callbacks, titles, decos);
        return false;
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
        // The rule itself is the line's bottom border, so the `---` markers are
        // syntax like any other: hiding them off the active line is what keeps the
        // rendered rule from reading as a stray `---` sitting above a line.
        decos.push(Decoration.line({ class: 'cm-atlas-hr' }).range(view.state.doc.lineAt(node.from).from));
        if (!activeLines.has(lineNumberAt(view, node.from))) {
          decos.push(hideDeco.range(node.from, node.to));
        }
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

/** Standard markdown link `[text](url)`. Off active line it renders as a safe anchor; on active line it stays raw. */
function decorateLink(
  view: EditorView,
  node: SyntaxNode,
  activeLines: Set<number>,
  callbacks: LivePreviewCallbacks,
  titles: Record<string, string>,
  decos: Range<Decoration>[],
): void {
  const lineNo = lineNumberAt(view, node.from);
  if (activeLines.has(lineNo)) {
    decos.push(Decoration.mark({ class: 'cm-atlas-link' }).range(node.from, node.to));
    return;
  }

  const marks = collectChildren(node, 'LinkMark');
  const url = findChild(node.firstChild, 'URL');
  const open = marks[0];
  const closeText = marks[1];

  if (open && closeText && url) {
    const text = view.state.doc.sliceString(open.to, closeText.from);
    const href = view.state.doc.sliceString(url.from, url.to);
    decos.push(
      Decoration.replace({
        widget: new LinkWidget(text, href, { titles, onWikilinkClick: callbacks.onWikilinkClick }),
      }).range(node.from, node.to),
    );
  }
}

function decorateAutolink(
  view: EditorView,
  node: SyntaxNode,
  activeLines: Set<number>,
  callbacks: LivePreviewCallbacks,
  titles: Record<string, string>,
  decos: Range<Decoration>[],
): void {
  const lineNo = lineNumberAt(view, node.from);
  if (activeLines.has(lineNo)) {
    decos.push(Decoration.mark({ class: 'cm-atlas-link' }).range(node.from, node.to));
    return;
  }

  const url = findChild(node.firstChild, 'URL');
  if (url) {
    const href = view.state.doc.sliceString(url.from, url.to);
    decos.push(
      Decoration.replace({
        widget: new LinkWidget(href, href, { titles, onWikilinkClick: callbacks.onWikilinkClick }),
      }).range(node.from, node.to),
    );
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

function decorateInlineMath(
  view: EditorView,
  from: number,
  to: number,
  activeLines: Set<number>,
  decos: Range<Decoration>[],
  blockRanges: BlockRange[],
  mathRanges: MathRange[],
): void {
  const doc = view.state.doc;
  const { start, end } = overlappingRangeBounds(mathRanges, from, to);

  for (let i = start; i < end; i += 1) {
    const range = mathRanges[i];
    if (range === undefined || range.kind !== 'inline') continue;
    if (isInsideBlock(range.from, blockRanges)) continue;

    const lineNo = doc.lineAt(range.from).number;
    if (activeLines.has(lineNo)) continue;

    decos.push(
      Decoration.replace({ widget: new MathInlineWidget(mathBody(doc, range)) }).range(range.from, range.to),
    );
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
  wikilinkRanges: WikilinkRange[],
): void {
  const { start, end } = overlappingRangeBounds(wikilinkRanges, from, to);

  for (let i = start; i < end; i += 1) {
    const range = wikilinkRanges[i];
    if (range === undefined) continue;

    // Skip wikilinks inside a block-replaced range (e.g. a rendered table cell):
    // a replace inside an already-replaced block would overlap and throw.
    if (isInsideBlock(range.from, blockRanges)) continue;

    const lineNo = lineNumberAt(view, range.from);

    if (activeLines.has(lineNo)) {
      decos.push(Decoration.mark({ class: 'cm-atlas-wikilink-raw' }).range(range.from, range.to));
      continue;
    }

    const ref = parseWikilinkInner(range.inner);
    const display = wikilinkDisplay(ref, titles);
    decos.push(
      Decoration.replace({ widget: new WikilinkWidget(ref, display, callbacks.onWikilinkClick) }).range(
        range.from,
        range.to,
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

/** True when `[from, to]` lies within one of the sorted, non-overlapping `ranges`. */
function isInsideRange(from: number, to: number, ranges: readonly PositionRange[]): boolean {
  const range = rangeContaining(ranges, from);
  return range !== null && to <= range.to;
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
 * Reading mode (`reveal` false) additionally collapses every paragraph's soft line
 * breaks, which is what lets a hard-wrapped source reflow to the container width.
 * Edit modes never do: joining lines would move the caret away from its source
 * position.
 *
 * Exported for unit testing the block-discovery walk without a DOM.
 */
export function buildBlockDecorations(
  state: EditorState,
  reveal: boolean,
  ctx: InlineCtx,
  tree: ReturnType<typeof syntaxTree> = syntaxTree(state),
  /** Precomputed math ranges; when omitted, scans `state.doc` once. */
  mathRanges?: MathRange[],
): DecorationSet {
  return collectBlockDecorations(state, reveal, ctx, tree, mathRanges).decorations;
}

/**
 * The line span of a block-widget candidate (table, HTML block, mermaid fence,
 * block math), as document positions of its first line's start and last line's
 * end. A selection range touching this span reveals the block.
 */
interface BlockSpan {
  lineFrom: number;
  lineTo: number;
}

interface BuiltBlocks {
  decorations: DecorationSet;
  /** Every candidate block, whether or not it is currently revealed. */
  blocks: BlockSpan[];
}

/** True when `selection` touches any line of `block`, matching the active-block rule. */
function blockRevealed(block: BlockSpan, selection: EditorSelection): boolean {
  return selection.ranges.some((range) => range.from <= block.lineTo && range.to >= block.lineFrom);
}

/**
 * True when moving the selection from `before` to `after` neither reveals nor
 * re-collapses any block, so the existing block decorations remain valid.
 */
export function revealedBlocksUnchanged(
  blocks: readonly BlockSpan[],
  before: EditorSelection,
  after: EditorSelection,
): boolean {
  return blocks.every((block) => blockRevealed(block, before) === blockRevealed(block, after));
}

function collectBlockDecorations(
  state: EditorState,
  reveal: boolean,
  ctx: InlineCtx,
  tree: ReturnType<typeof syntaxTree>,
  mathRanges?: MathRange[],
): BuiltBlocks {
  const doc = state.doc;
  const activeLines = activeLinesFromSelection(state, reveal);
  const decos: Range<Decoration>[] = [];
  const blocks: BlockSpan[] = [];

  const spanOf = (from: number, to: number): { firstLine: number; lastLine: number } => {
    const first = doc.lineAt(from);
    const last = doc.lineAt(to);
    blocks.push({ lineFrom: first.from, lineTo: last.to });
    return { firstLine: first.number, lastLine: last.number };
  };

  const blockReplace = (node: SyntaxNode, widget: WidgetType): void => {
    decos.push(Decoration.replace({ widget, block: true }).range(node.from, node.to));
  };

  const rangeReplace = (range: MathRange, widget: WidgetType): void => {
    decos.push(Decoration.replace({ widget, block: true }).range(range.from, range.to));
  };

  const ranges = mathRanges ?? findMathRanges(doc.toString());
  for (const range of ranges) {
    if (range.kind !== 'block') continue;
    const { firstLine, lastLine } = spanOf(range.from, range.to);
    if (!isBlockActive(firstLine, lastLine, activeLines)) {
      rangeReplace(range, new MathBlockWidget(mathBody(doc, range), range.from));
    }
  }

  tree.iterate({
    enter: (node) => {
      if (INLINE_ONLY_BLOCKS.has(node.name)) {
        if (!reveal && node.name === 'Paragraph') {
          const source = doc.sliceString(node.from, node.to);
          for (const range of paragraphSoftBreaks(source, node.from)) {
            decos.push(softBreakDeco.range(range.from, range.to));
          }
        }
        return false;
      }

      if (node.name === 'Table') {
        const { firstLine, lastLine } = spanOf(node.from, node.to);
        if (!isBlockActive(firstLine, lastLine, activeLines)) {
          const parsed = parseTable(doc.sliceString(node.from, node.to));
          if (parsed !== null) blockReplace(node.node, new TableWidget(parsed, node.from, ctx));
        }
        return false;
      }

      if (node.name === 'HTMLBlock') {
        const { firstLine, lastLine } = spanOf(node.from, node.to);
        if (!isBlockActive(firstLine, lastLine, activeLines)) {
          blockReplace(node.node, new HtmlBlockWidget(doc.sliceString(node.from, node.to), node.from));
        }
        return false;
      }

      if (node.name === 'FencedCode') {
        if (fencedLanguage(state, node.node) === 'mermaid') {
          const { firstLine, lastLine } = spanOf(node.from, node.to);
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
  return { decorations: builder.finish(), blocks };
}

/**
 * Block-decoration field state: decorations, the candidate blocks they were
 * built from, and a cached full-doc math scan, so pure selection moves can tell
 * whether any block changed state without rebuilding or re-stringifying.
 */
interface BlockFieldState {
  decorations: DecorationSet;
  blocks: BlockSpan[];
  mathRanges: MathRange[];
}

/**
 * StateField that provides the block decorations.
 *
 * Full-document `ensureSyntaxTree` runs only on create, so the first paint shows
 * blocks past the init parse. Afterwards the field reads the tree the background
 * parser delivers: every parser progress transaction and document change rebuilds
 * from `syntaxTree(state)`, and a selection move rebuilds only when it reveals or
 * re-collapses a block.
 */
function blockDecorationsField(reveal: boolean, ctx: InlineCtx): StateField<BlockFieldState> {
  return StateField.define<BlockFieldState>({
    create(state) {
      // Read the tree ensureSyntaxTree RETURNS: it advances the parse but leaves
      // the Language state field on the short init tree, so syntaxTree(state)
      // alone would miss block widgets (tables, diagrams) past the first ~3 KB.
      const tree = ensureSyntaxTree(state, state.doc.length, VIEWPORT_PARSE_BUDGET_MS) ?? syntaxTree(state);
      const mathRanges = findMathRanges(state.doc.toString());
      return { ...collectBlockDecorations(state, reveal, ctx, tree, mathRanges), mathRanges };
    },
    update(value, tr) {
      const treeChanged = syntaxTree(tr.startState) !== syntaxTree(tr.state);

      if (tr.docChanged || treeChanged) {
        const mathRanges = tr.docChanged ? findMathRanges(tr.state.doc.toString()) : value.mathRanges;
        return {
          ...collectBlockDecorations(tr.state, reveal, ctx, syntaxTree(tr.state), mathRanges),
          mathRanges,
        };
      }

      if (tr.selection !== undefined) {
        if (!reveal || revealedBlocksUnchanged(value.blocks, tr.startState.selection, tr.state.selection)) {
          return value;
        }
        return {
          ...collectBlockDecorations(tr.state, reveal, ctx, syntaxTree(tr.state), value.mathRanges),
          mathRanges: value.mathRanges,
        };
      }

      return value;
    },
    provide: (field) => EditorView.decorations.from(field, (v) => v.decorations),
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
      atomic: DecorationSet;
      private rangeCache: DocRangeCache;

      constructor(view: EditorView) {
        // buildDecorations forces the parse up to the viewport and reads the
        // resulting tree (see viewportSyntaxTree), so the very first paint of a
        // large document is already fully decorated instead of showing raw
        // markdown until a background-parse transaction arrives.
        this.rangeCache = buildDocRangeCache(view.state.doc.toString());
        const built = buildDecorations(view, callbacks, reveal, titles, this.rangeCache);
        this.decorations = built.decorations;
        this.atomic = built.atomic;
      }

      update(update: ViewUpdate): void {
        // Rebuild on the obvious triggers, and ALSO when the syntax tree changed:
        // the parser dispatches tree-progress transactions that carry none of the
        // doc/selection/viewport flags, and skipping them is what made decorations
        // appear only after the first click.
        const treeChanged = syntaxTree(update.startState) !== syntaxTree(update.state);
        if (update.docChanged || update.selectionSet || update.viewportChanged || treeChanged) {
          if (
            shouldRefreshDocRangeCache({
              docChanged: update.docChanged,
              syntaxTreeChanged: treeChanged,
            })
          ) {
            this.rangeCache = buildDocRangeCache(update.state.doc.toString());
          }

          const built = buildDecorations(update.view, callbacks, reveal, titles, this.rangeCache);
          this.decorations = built.decorations;
          this.atomic = built.atomic;
        }
      }
    },
    {
      decorations: (plugin) => plugin.decorations,
      // Only the replaced/hidden ranges are atomic — never the visible styling
      // marks — so inline code, emphasis and links stay editable (caret can enter
      // them, and a single backspace deletes one character, not the whole span).
      provide: (plugin) =>
        EditorView.atomicRanges.of((view) => view.plugin(plugin)?.atomic ?? Decoration.none),
    },
  );

  const ctx: InlineCtx = { titles, onWikilinkClick: callbacks.onWikilinkClick };
  return [inline, blockDecorationsField(reveal, ctx)];
}
