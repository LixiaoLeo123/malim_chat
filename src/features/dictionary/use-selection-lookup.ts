import { useEffect, useRef } from "react";
import { useEvent } from "../../hooks/use-event";

/**
 * Watches document selection changes and, after a short debounce, reports a fresh
 * non-collapsed selection that overlaps `bodyRef` through `onSelect`. Used for the
 * mouse/keyboard path of the dictionary lookup. `onSelect` fires at most once per
 * selection gesture; a collapsed selection resets the gate.
 */
export function useSelectionLookup(bodyRef: React.RefObject<HTMLElement | null>, lookupActive: boolean, onSelect: (anchor: { left?: number; bottom?: number }) => void) {
  const selectionCaptured = useRef(false);
  const lookupActiveRef = useRef(lookupActive);
  useEffect(() => { lookupActiveRef.current = lookupActive; }, [lookupActive]);
  const selectionTimer = useRef<number | null>(null);
  const onSelectionChange = useEvent(() => {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !selection.toString().trim()) {
      if (selectionTimer.current !== null) { window.clearTimeout(selectionTimer.current); selectionTimer.current = null; }
      selectionCaptured.current = false;
      return;
    }
    if (selectionCaptured.current || !bodyRef.current || !selection.containsNode(bodyRef.current, true)) return;
    if (selectionTimer.current !== null) { window.clearTimeout(selectionTimer.current); selectionTimer.current = null; }
    selectionTimer.current = window.setTimeout(() => {
      selectionTimer.current = null;
      if (lookupActiveRef.current) return;
      const latest = window.getSelection();
      if (selectionCaptured.current) return;
      if (latest && !latest.isCollapsed && latest.toString().trim() && bodyRef.current && latest.containsNode(bodyRef.current, true)) {
        selectionCaptured.current = true;
        const range = latest.rangeCount ? latest.getRangeAt(0).getBoundingClientRect() : undefined;
        onSelect({ left: range?.left, bottom: range?.bottom });
      }
    }, 200);
  });
  useEffect(() => { document.addEventListener("selectionchange", onSelectionChange); return () => { document.removeEventListener("selectionchange", onSelectionChange); if (selectionTimer.current !== null) window.clearTimeout(selectionTimer.current); }; }, [onSelectionChange]);
  return selectionCaptured;
}
