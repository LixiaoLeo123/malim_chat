import type { GenerationSettings, ToolActivity } from "../types";

export const now = () => new Date().toISOString();
export const defaultGeneration: GenerationSettings = { temperature: 0.7, reasoning_effort: "medium", enable_markdown: true, stream: true, context_rounds: null, tool_rounds: null, top_p: null, top_k: null, max_tokens: null, web_search_uses: null, web_fetch_uses: null };

export function uuid() {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  return [...bytes].map((value, index) => `${[4, 6, 8, 10].includes(index) ? "-" : ""}${value.toString(16).padStart(2, "0")}`).join("");
}

/**
 * Android raises its selection toolbar (Copy / Share / Define) from the same long press that
 * opens the dictionary popover, so the two fight. The packaged app finds the word through
 * `caretRangeFromPoint`, which keeps working without a native selection, so the app turns
 * native selection off and a browser does not.
 */
export const androidApp = /android/i.test(navigator.userAgent) && "__TAURI_INTERNALS__" in window;

// Closing the dictionary popover fires pointerdown, then pointerup and selectionchange
// land on the message text right under it; without a cooldown the same word immediately
// re-opens the popover the user just dismissed.
let lookupCooldownUntil = 0;
export function suppressLookup() { lookupCooldownUntil = Date.now() + 350; }
export function lookupSuppressed() { return Date.now() < lookupCooldownUntil; }

export async function copyText(text: string) { if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); }
  else {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.top = "-9999px";
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

export function updateToolEvents(current: ToolActivity[], next?: ToolActivity) {
  if (!next) return current;
  const index = current.findIndex((item) => item.id === next.id);
  if (index >= 0) return current.map((item, itemIndex) => itemIndex === index ? { ...item, ...next } : item);
  return [...current, next];
}

export function preprocessLatex(input: string): string {
  const blocks: string[] = [];
  const protectedText = input.replace(/```[\s\S]*?```|`[^`\n]*`/g, (match) => {
    blocks.push(match);
    return `\u0000${blocks.length - 1}\u0000`;
  });
  const converted = protectedText
    .replace(/\\\[([\s\S]*?)\\\]/g, (_, body) => `$$\n${body}\n$$`)
    .replace(/\\\(([\s\S]*?)\\\)/g, (_, body) => `$${body}$`);
  // The NUL sentinel cannot collide with LaTeX the user is shown.
  // eslint-disable-next-line no-control-regex
  return converted.replace(/\u0000(\d+)\u0000/g, (_, index) => blocks[Number(index)]);
}

export function sourceHost(url: string) {
  try { return new URL(url).hostname.replace(/^www\./, ""); }
  catch { return url; }
}

export function splitLegacyThinking(value: string) {
  const reasoning: string[] = [];
  const content = value.replace(/<(think|thinking)>([\s\S]*?)(?:<\/\1>|$)/gi, (_, _tag, thought: string) => {
    if (thought.trim()) reasoning.push(thought.trim());
    return "";
  });
  return { content: content.trimStart(), reasoning: reasoning.join("\n") };
}

