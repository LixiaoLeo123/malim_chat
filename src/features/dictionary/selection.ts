type CSSGlobal = { CSS: { highlights?: Map<string, unknown> } };

function highlightStore() {
  return (globalThis as unknown as CSSGlobal).CSS?.highlights;
}export function applyLookupHighlight(node: Node, start: number, end: number) {
  try {
    const store = highlightStore();
    const HighlightCtor = (window as unknown as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
    if (!store || !HighlightCtor) return;
    const range = new Range();
    range.setStart(node, start);
    range.setEnd(node, end);
    store.set("dictionary-lookup", new HighlightCtor(range));
  } catch { /* highlight API unavailable */ }
}

export function clearLookupHighlight() {
  try { highlightStore()?.delete("dictionary-lookup"); } catch { /* noop */ }
}

/** Word characters matched by the dictionary lookup (Latin, Cyrillic, diacritics). */
export const WORD_CHARS = /[A-Za-zÀ-ɏͰ-ԯ'’]/;

export function extractWordAtPoint(x: number, y: number, container: HTMLElement | null): { word: string; node: Node; start: number; end: number } | null {
  const range = document.caretRangeFromPoint ? document.caretRangeFromPoint(x, y) : null;
  const node = range?.startContainer;
  if (!range || !node || node.nodeType !== Node.TEXT_NODE || !container?.contains(node)) return null;
  const text = node.textContent ?? "";
  const isWord = (char?: string) => char ? WORD_CHARS.test(char) : false;
  const offset = range.startOffset;
  if (!isWord(text[offset - 1]) && !isWord(text[offset])) return null;
  let start = offset;
  while (start > 0 && isWord(text[start - 1])) start--;
  let end = offset;
  while (end < text.length && isWord(text[end])) end++;
  const word = text.slice(start, end);
  if (!word || word.length > 120) return null;
  return { word, node, start, end };
}

export function clampAnchor(x: number, y: number) {
  return {
    x: Math.min(window.innerWidth - 18, Math.max(18, x)),
    y: Math.min(window.innerHeight - 18, Math.max(18, y)),
  };
}
