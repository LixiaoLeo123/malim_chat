import type { ClipboardEvent } from "react";

/**
 * KaTeX renders formulas as HTML spans, so a plain browser copy yields mangled math.
 * When the selection crosses a formula, rewrite the clipboard text with each formula
 * back as its raw TeX (KaTeX keeps it in the mathml annotation for screen readers),
 * wrapped per display mode and blended into the surrounding text.
 */
export function handleKatexCopy(event: ClipboardEvent<HTMLDivElement>, bodyRef: React.RefObject<HTMLDivElement | null>, renderMarkdown: boolean) {
  if (!renderMarkdown) return;
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return;
  const range = selection.getRangeAt(0);
  // The common ancestor of a formula-only selection sits inside .katex, where no
  // .katex can be found again — always search from the message body.
  const container = bodyRef.current;
  if (!container || !range.intersectsNode(container)) return;
  if (!container.querySelector(".katex")) return;
  // Formulas the selection cuts into contribute no mathml annotation to the clone (it
  // sits before the visible html where the selection starts), so the TeX is read from
  // the live elements, matched by document order to the cloned fragments.
  const live = Array.from(container.querySelectorAll(".katex")).filter((el) => range.intersectsNode(el));
  if (!live.length) return;
  const holder = document.createElement("div");
  holder.appendChild(range.cloneContents());
  const fragments = Array.from(holder.querySelectorAll(".katex"));
  fragments.forEach((math, index) => {
    const source = live[index] ?? math;
    const tex = source.querySelector(".katex-mathml annotation")?.textContent ?? "";
    const block = source.closest(".katex-display") !== null;
    math.replaceWith(document.createTextNode(block ? `$$${tex}$$` : `$${tex}$`));
  });
  event.clipboardData?.setData("text/plain", holder.textContent ?? "");
  event.preventDefault();
}

/**
 * Rendered formulas are overlapping positioned spans, so the browser cannot build a
 * selection inside one — dragging highlights nothing until it swallows the whole atom.
 * Treat every formula as one selectable unit: a press inside it selects the whole
 * formula, which then copies as raw TeX through handleKatexCopy.
 */
export function handleMathMouseDown(event: React.MouseEvent<HTMLDivElement>, bodyRef: React.RefObject<HTMLDivElement | null>, renderMarkdown: boolean) {
  if (!renderMarkdown || event.button !== 0) return;
  const target = event.target as HTMLElement;
  const math = target.closest(".katex-display") ?? target.closest(".katex");
  if (!math || !bodyRef.current?.contains(math)) return;
  event.preventDefault();
  const selection = window.getSelection();
  if (!selection) return;
  const range = document.createRange();
  range.selectNodeContents(math);
  selection.removeAllRanges();
  selection.addRange(range);
}
