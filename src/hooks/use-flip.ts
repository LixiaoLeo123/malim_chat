import { useLayoutEffect, useRef } from "react";

/**
 * FLIP animation: record element positions before a reorder, then play the delta as a
 * transform so rows glide instead of jumping. The caller forwards its own dependency
 * list, so React cannot check it statically.
 */
export function useFlip(ref: { current: HTMLElement | null }, deps: unknown[]) {
  const previous = useRef<Map<string, DOMRect>>(new Map());
  useLayoutEffect(() => {
    const container = ref.current;
    if (!container) return;
    const current = new Map<string, DOMRect>();
    const nodes = new Map<string, HTMLElement>();
    container.querySelectorAll<HTMLElement>("[data-flip-id]").forEach((node) => {
      const id = node.dataset.flipId ?? "";
      current.set(id, node.getBoundingClientRect());
      nodes.set(id, node);
    });
    const moved: HTMLElement[] = [];
    for (const [id, rect] of previous.current) {
      // The node is already in hand from the measurement pass; a per-id querySelector
      // here makes reordering O(n²) and visibly slow once the history grows.
      const node = nodes.get(id);
      const now = current.get(id);
      if (node && now && (Math.abs(now.top - rect.top) > 1 || Math.abs(now.left - rect.left) > 1)) {
        node.style.transition = "none";
        node.style.transform = `translate(${rect.left - now.left}px, ${rect.top - now.top}px)`;
        moved.push(node);
      }
    }
    previous.current = current;
    if (!moved.length) return;
    requestAnimationFrame(() => { moved.forEach((node) => { node.style.transition = "transform 300ms cubic-bezier(.2,.7,.2,1)"; node.style.transform = ""; }); const clear = (event: TransitionEvent) => { const target = event.target as HTMLElement; target.style.transition = ""; target.style.transform = ""; target.removeEventListener("transitionend", clear); }; moved.forEach((node) => node.addEventListener("transitionend", clear)); });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
