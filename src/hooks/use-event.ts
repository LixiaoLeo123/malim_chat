import { useLayoutEffect, useRef, useCallback } from "react";

/**
 * A callback with a stable identity that always runs the latest closure. Replaces the
 * `useCallback(fn, [deps])` wrappers, which kept the first render's closure alive.
 */
export function useEvent<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const ref = useRef(fn);
  useLayoutEffect(() => { ref.current = fn; });
  return useCallback((...args: A) => ref.current(...args), []);
}
