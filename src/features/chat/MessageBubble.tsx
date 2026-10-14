import { memo, useEffect, useRef, useState, type ClipboardEvent, type TouchEvent } from "react";
import { Bot, Check, Copy, Edit3, Trash2 } from "lucide-react";
import type { Message } from "../../types";
import { copyText, lookupSuppressed, splitLegacyThinking } from "../../lib/utils";
import { handleKatexCopy, handleMathMouseDown } from "../../lib/katex-copy";
import { applyLookupHighlight } from "../dictionary/selection";
import { useSelectionLookup } from "../dictionary/use-selection-lookup";
import { MemoMarkdown } from "../../components/common/markdown";
import { SourceList } from "./SourceList";
import { ToolActivityList } from "./ToolActivityList";

function MessageBubble({ message, markdownEnabled, theme, isLast, lookupActive, onEdit, onDelete, onRetry, onLookup }: { message: Message; markdownEnabled: boolean; theme: "light" | "dark"; isLast: boolean; lookupActive: boolean; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void }) {
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [copied, setCopied] = useState(false);
  useEffect(() => { setDraft(message.content); }, [message.content]);
  const bodyRef = useRef<HTMLDivElement | null>(null);

  // --- Dictionary lookup: touch long-press path --------------------------------
  const selectionCaptured = useRef(false);
  const longPressTimer = useRef<number | null>(null);
  const longPress = useRef<{ x: number; y: number } | null>(null);
  const cancelLongPressScroll = useRef<(() => void) | null>(null);
  function cancelLongPress() {
    if (longPressTimer.current !== null) { window.clearTimeout(longPressTimer.current); longPressTimer.current = null; }
    longPress.current = null;
    if (cancelLongPressScroll.current) { cancelLongPressScroll.current(); cancelLongPressScroll.current = null; }
  }
  function lookupAt(x: number, y: number) {
    const range = document.caretRangeFromPoint ? document.caretRangeFromPoint(x, y) : null;
    const node = range?.startContainer;
    if (!range || !node || node.nodeType !== Node.TEXT_NODE || !bodyRef.current?.contains(node)) return;
    const text = node.textContent ?? "";
    const isWord = (char?: string) => char ? /[A-Za-zÀ-ɏͰ-ԯ'’]/.test(char) : false;
    const offset = range.startOffset;
    if (!isWord(text[offset - 1]) && !isWord(text[offset])) return;
    let start = offset;
    while (start > 0 && isWord(text[start - 1])) start--;
    let end = offset;
    while (end < text.length && isWord(text[end])) end++;
    const word = text.slice(start, end);
    if (word && word.length <= 120 && !lookupSuppressed()) { selectionCaptured.current = true; applyLookupHighlight(node, start, end); onLookup(word, { x: Math.min(window.innerWidth - 18, Math.max(18, x)), y: Math.min(window.innerHeight - 18, Math.max(18, y + 12)) }); }
  }
  function handleTouchStart(event: TouchEvent<HTMLDivElement>) {
    if ((event.target as HTMLElement).closest?.("button, a, textarea, input, select")) return;
    const touch = event.touches[0];
    if (!touch) return;
    cancelLongPress();
    const point = { x: touch.clientX, y: touch.clientY };
    longPress.current = point;
    const onScroll = () => cancelLongPress();
    window.addEventListener("scroll", onScroll, true);
    cancelLongPressScroll.current = () => window.removeEventListener("scroll", onScroll, true);
    longPressTimer.current = window.setTimeout(() => {
      const target = longPress.current;
      if (!target) return;
      cancelLongPress();
      lookupAt(target.x, target.y);
    }, 450);
  }
  function handleTouchMove(event: TouchEvent<HTMLDivElement>) {
    const point = longPress.current;
    const touch = event.touches[0];
    if (point && touch && (Math.abs(touch.clientX - point.x) > 10 || Math.abs(touch.clientY - point.y) > 10)) cancelLongPress();
  }
  function handleTouchEnd() { cancelLongPress(); }

  // --- Dictionary lookup: selection path --------------------------------------
  const captureSelection = (anchor?: { left?: number; bottom?: number }) => {
    if (lookupActive || lookupSuppressed()) return;
    const selected = window.getSelection()?.toString().trim() ?? "";
    if (selected && selected.length <= 120) {
      selectionCaptured.current = true;
      // Prefer the live selection's rect: the debounce path may hand down a stale one.
      const range = window.getSelection()?.rangeCount ? window.getSelection()?.getRangeAt(0).getBoundingClientRect() : undefined;
      onLookup(selected, {
        x: Math.min(window.innerWidth - 18, Math.max(18, range?.left ?? anchor?.left ?? window.innerWidth / 2)),
        y: Math.min(window.innerHeight - 18, Math.max(18, range?.bottom ?? anchor?.bottom ?? window.innerHeight / 2)),
      });
    }
  };
  useSelectionLookup(bodyRef, lookupActive, captureSelection);

  // --- Copy ---------------------------------------------------------------------
  async function copyContent() {
    try {
      await copyText(content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch { /* clipboard unavailable */ }
  }
  function onCopy(event: ClipboardEvent<HTMLDivElement>) { handleKatexCopy(event, bodyRef, renderMarkdown); }
  function onBodyMouseDown(event: React.MouseEvent<HTMLDivElement>) { handleMathMouseDown(event, bodyRef, renderMarkdown); }

  const legacy = splitLegacyThinking(message.content);
  const content = legacy.content;
  const reasoning = [message.reasoning_content, legacy.reasoning].filter(Boolean).join("\n");
  const renderMarkdown = markdownEnabled && message.role === "assistant" && message.content_format === "markdown";
  const isUser = message.role === "user";
  const kind = isUser ? "user" : message.role === "summary" ? "summary" : "assistant";
  return <article id={`message-${message.id}`} className={`message ${kind} ${message.status === "error" ? "message-error" : ""}`}>{isUser ? <div className="message-body" ref={bodyRef} onMouseUp={() => captureSelection({})} onCopy={onCopy} onMouseDown={onBodyMouseDown} onTouchStart={handleTouchStart} onTouchMove={handleTouchMove} onTouchEnd={handleTouchEnd} onTouchCancel={cancelLongPress}>{editing ? <div className="edit-box"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} /><button className="primary small" type="button" onClick={() => { void onEdit(message, draft); setEditing(false); }}>Save</button><button className="text-button small" type="button" onClick={() => { setDraft(message.content); setEditing(false); }}>Cancel</button></div> : <><div className={`message-content ${renderMarkdown ? "markdown-content" : "plain-content"} user-bubble`}>{renderMarkdown ? <MemoMarkdown content={content} theme={theme} /> : content}</div>{message.images?.length > 0 && <div className="message-images">{message.images.map((src) => <img key={src} src={src} alt="Attached image" />)}</div>}</>}</div> : <><div className="message-avatar"><Bot size={15} /></div><div className="message-body" ref={bodyRef} onMouseUp={() => captureSelection({})} onCopy={onCopy} onMouseDown={onBodyMouseDown} onTouchStart={handleTouchStart} onTouchMove={handleTouchMove} onTouchEnd={handleTouchEnd} onTouchCancel={cancelLongPress}>{editing ? <div className="edit-box"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} /><button className="primary small" type="button" onClick={() => { void onEdit(message, draft); setEditing(false); }}>Save</button><button className="text-button small" type="button" onClick={() => { setDraft(message.content); setEditing(false); }}>Cancel</button></div> : <><ToolActivityList events={message.tool_events} reasoning={reasoning} streaming={message.status === "streaming"} theme={theme} markdown={markdownEnabled} />{message.status === "streaming" && !content && !reasoning ? <span className="typing"><i /><i /><i /></span> : (content || message.images?.length > 0) && <div className={`message-content ${renderMarkdown ? "markdown-content" : "plain-content"}`}>{renderMarkdown ? <MemoMarkdown content={content} theme={theme} /> : content}</div>}</>} {isLast && message.status === "error" && (message.role === "user" || message.retry_message_id) && <button type="button" className="retry-button" onClick={() => void onRetry(message)}>Retry response</button>}<SourceList sources={message.search_sources ?? []} /><div className="message-tools"><button type="button" className={copied ? "copied" : ""} title={copied ? "Copied" : "Copy"} onClick={() => void copyContent()}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>{!message.optimistic && message.role !== "system" && <button type="button" title="Edit" onClick={() => setEditing(true)}><Edit3 size={14} /></button>}<button type="button" className={confirmDelete ? "delete armed" : "delete"} title={confirmDelete ? "Click again to delete" : "Delete"} aria-label={confirmDelete ? "Confirm delete message" : "Delete message"} onBlur={() => setConfirmDelete(false)} onClick={(event) => { if (!confirmDelete) { event.preventDefault(); event.currentTarget.focus(); setConfirmDelete(true); return; } event.preventDefault(); setConfirmDelete(false); void onDelete(message); }}><Trash2 size={14} /></button>{message.model && message.status !== "streaming" && message.status !== "pending" && <span className="message-model">{message.model}</span>}</div></div></>}</article>;
}
export const MemoMessageBubble = memo(MessageBubble);
