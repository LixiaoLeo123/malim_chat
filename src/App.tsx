import { ClipboardEvent, Component, FormEvent, memo, ReactNode, TouchEvent, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { Bot, Check, ChevronDown, ChevronsUpDown, CircleAlert, Copy, Edit3, ImagePlus, LoaderCircle, LogOut, Menu, MessageSquare, PanelLeftClose, PanelLeftOpen, Search, Settings2, Square, SquarePen, Star, Trash2, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark, oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";
import "./additions.css";
import { api, setSession, subscribeSession, type StreamEvent } from "./api";
import { useAppStore } from "./store";
import type { Conversation, DictionaryResponse, GenerationSettings, Message, Provider, ProviderKind, ProviderModel, Session, ToolActivity } from "./types";

const now = () => new Date().toISOString();
const defaultGeneration: GenerationSettings = { temperature: 0.7, reasoning_effort: "medium", enable_markdown: true, stream: true, context_rounds: null, tool_rounds: null };
function uuid() {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  return [...bytes].map((value, index) => `${[4, 6, 8, 10].includes(index) ? "-" : ""}${value.toString(16).padStart(2, "0")}`).join("");
}
async function copyText(text: string) {
  if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); }
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

/**
 * A callback with a stable identity that always runs the latest closure. Replaces the
 * `useCallback(fn, [deps])` wrappers, which kept the first render's closure alive.
 */
function useEvent<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const ref = useRef(fn);
  useLayoutEffect(() => { ref.current = fn; });
  return useCallback((...args: A) => ref.current(...args), []);
}

function updateToolEvents(current: ToolActivity[], next?: ToolActivity) {
  if (!next) return current;
  const index = current.findIndex((item) => item.id === next.id);
  if (index >= 0) return current.map((item, itemIndex) => itemIndex === index ? { ...item, ...next } : item);
  return [...current, next];
}

export function App() {
  const { session, setSession: saveSession } = useAppStore();
  useEffect(() => {
    const unsubscribe = subscribeSession(saveSession);
    const raw = localStorage.getItem("malim-session");
    if (raw) try { setSession(JSON.parse(raw) as Session); } catch { setSession(null); }
    return unsubscribe;
  }, [saveSession]);
  if (!session) return <Auth onSession={setSession} />;
  return <ChatErrorBoundary><Chat /></ChatErrorBoundary>;
}

class ChatErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error) { console.error("Chat render failure", error); }
  render() {
    if (this.state.failed) return <main className="recovery-screen"><div><Bot size={28} /><h1>Unable to render this chat</h1><button className="primary" onClick={() => window.location.reload()}>Reload chat</button></div></main>;
    return this.props.children;
  }
}

function Auth({ onSession }: { onSession: (session: Session) => void }) {
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try { onSession(mode === "login" ? await api.login(email, password) : await api.signup(email, password, name)); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to continue."); }
    finally { setBusy(false); }
  }
  return <main className="auth-shell"><section className="auth-brand"><div className="brand-mark"><Bot size={28} /></div><h1>malim_chat</h1><p>One secure place for the AI models your work depends on.</p></section><form className="auth-form" onSubmit={submit}><h2>{mode === "login" ? "Welcome back" : "Create your account"}</h2>{mode === "signup" && <label>Display name<input value={name} required maxLength={80} onChange={(event) => setName(event.target.value)} /></label>}<label>Email<input type="email" value={email} required autoComplete="email" onChange={(event) => setEmail(event.target.value)} /></label><label>Password<input type="password" value={password} minLength={12} required autoComplete={mode === "login" ? "current-password" : "new-password"} onChange={(event) => setPassword(event.target.value)} /></label>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}{mode === "login" ? "Sign in" : "Create account"}</button><button type="button" className="text-button" onClick={() => { setMode(mode === "login" ? "signup" : "login"); setError(null); }}>{mode === "login" ? "Need an account? Sign up" : "Already have an account? Sign in"}</button></form></main>;
}

function Chat() {
  const {
    session, conversations, providers, activeId, messages, sidebarOpen, busy, error,
    appendConversations, prependMessages, setProviders, setConversations, setActiveId,
    setSidebarOpen, setBusy, setError, setMessages, upsertMessage, removeMessage,
  } = useAppStore();
  const [providerOpen, setProviderOpen] = useState(false);
  const [searchEnabled, setSearchEnabled] = useState(false);
  const [lookup, setLookup] = useState<{ word: string; x: number; y: number } | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [modelChanging, setModelChanging] = useState(false);
  const [generation, setGeneration] = useState<GenerationSettings>(defaultGeneration);
  const generationTimers = useRef<Record<string, number>>({});
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [compacting, setCompacting] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const streamAbort = useRef<AbortController | null>(null);
  const [conversationCursor, setConversationCursor] = useState<string | null>(null);
  const [loadingConversations, setLoadingConversations] = useState(false);
  const [messageCursors, setMessageCursors] = useState<Record<string, string | null>>({});
  const [loadingOlder, setLoadingOlder] = useState(false);
  const activeConversation = activeId ? conversations.find((item) => item.id === activeId) ?? null : null;

  const bootstrap = useEvent(async () => {
    try {
      const [chats, configured] = await Promise.all([api.conversations(), api.providers()]);
      setConversations(chats.items);
      setConversationCursor(chats.next_cursor);
      setProviders(configured);
      if (!useAppStore.getState().activeId && chats.items[0]) setActiveId(chats.items[0].id);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load your workspace."); }
  });
  const loadMessages = useEvent(async (id: string) => {
    try {
      const page = await api.messages(id);
      setMessages(id, page.items);
      setMessageCursors((current) => ({ ...current, [id]: page.next_cursor }));
    }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load messages."); }
  });
  useEffect(() => { void bootstrap(); }, [bootstrap]);
  useEffect(() => { if (activeId && !useAppStore.getState().messages[activeId]) void loadMessages(activeId); }, [activeId, loadMessages]);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => setTheme(media.matches ? "dark" : "light");
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, []);
  useEffect(() => {
    const conversation = useAppStore.getState().conversations.find((item) => item.id === activeId);
    if (conversation) setGeneration(conversation.generation_settings ?? defaultGeneration);
  }, [activeId]);
  useEffect(() => () => { Object.values(generationTimers.current).forEach((timer) => window.clearTimeout(timer)); }, []);

  const loadMoreConversations = useCallback(async function loadMoreConversations() {
    if (!conversationCursor || loadingConversations) return;
    setLoadingConversations(true);
    try {
      const page = await api.conversations(conversationCursor);
      appendConversations(page.items);
      setConversationCursor(page.next_cursor);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load conversations."); }
    finally { setLoadingConversations(false); }
  }, [appendConversations, conversationCursor, loadingConversations, setError]);
  async function loadOlderMessages(id: string) {
    const cursor = messageCursors[id];
    if (!cursor || loadingOlder) return;
    setLoadingOlder(true);
    try {
      const page = await api.messages(id, cursor);
      prependMessages(id, page.items);
      setMessageCursors((current) => ({ ...current, [id]: page.next_cursor }));
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load older messages."); }
    finally { setLoadingOlder(false); }
  }
  async function newChat() {
    if (!providers.length) { setProviderOpen(true); setError("Add a provider before creating a chat."); return; }
    try {
      const conversation = await api.createConversation({ provider_id: providers[0].id, model: providers[0].models[0]?.model ?? providers[0].default_model });
      setConversations([conversation, ...useAppStore.getState().conversations]);
      setMessages(conversation.id, []);
      setActiveId(conversation.id);
      setSidebarCollapsed(false);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to create a conversation."); }
  }
  async function changeModel(providerId: string, model: string) {
    if (!activeConversation || !model.trim()) return;
    setModelChanging(true);
    try {
      const updated = await api.updateConversation(activeConversation.id, { provider_id: providerId, model: model.trim() });
      setConversations(useAppStore.getState().conversations.map((item) => item.id === updated.id ? updated : item));
      setModelMenuOpen(false);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to change the model."); }
    finally { setModelChanging(false); }
  }
  function signOut() { setSession(null); }
  function beginStream() { const controller = new AbortController(); streamAbort.current = controller; setStreaming(true); return controller; }
  function endStream() { streamAbort.current = null; setStreaming(false); }
  function stopStreaming() { streamAbort.current?.abort(); }
  function changeGeneration(next: GenerationSettings) {
    if (!activeConversation) return;
    const conversationId = activeConversation.id;
    setGeneration(next);
    setConversations(useAppStore.getState().conversations.map((conversation) => conversation.id === conversationId ? { ...conversation, generation_settings: next } : conversation));
    window.clearTimeout(generationTimers.current[conversationId]);
    generationTimers.current[conversationId] = window.setTimeout(() => { void api.updateConversation(conversationId, { generation_settings: next }).catch((cause) => setError(cause instanceof Error ? cause.message : "Unable to save conversation parameters.")); }, 350);
  }
  async function sendMessage(conversationId: string, content: string, search: boolean, options = generation, images: string[] = []) {
    const mutationId = uuid();
    const optimistic: Message = { id: `local-${mutationId}`, conversation_id: conversationId, sequence: Number.MAX_SAFE_INTEGER - Date.now(), client_mutation_id: mutationId, role: "user", content, images, reasoning_content: "", content_format: "markdown", status: "pending", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now(), optimistic: true };
    let sent: Message | null = null;
    let pendingId: string | null = null;
    const controller = options.stream ? beginStream() : null;
    upsertMessage(conversationId, optimistic);
    setBusy(true);
    try {
      sent = await api.createMessage(conversationId, content, mutationId, search, images);
      upsertMessage(conversationId, sent);
      const list = useAppStore.getState().conversations;
      if (list[0]?.id !== conversationId) {
        const item = list.find((conversation) => conversation.id === conversationId);
        if (item) setConversations([{ ...item, updated_at: now() }, ...list.filter((conversation) => conversation.id !== conversationId)]);
      }
      pendingId = `assistant-${mutationId}`;
      const turn = await runTurn(conversationId, sent, options, search, pendingId, [], controller?.signal);
      removeMessage(conversationId, pendingId);
      upsertMessage(conversationId, { ...turn.answer, tool_events: turn.toolEvents });
      const chats = await api.conversations();
      setConversations(chats.items);
      setConversationCursor(chats.next_cursor);
    } catch (cause) {
      if (pendingId) removeMessage(conversationId, pendingId);
      if ((cause as Error)?.name === "AbortError") return;
      upsertMessage(conversationId, { ...(sent ?? optimistic), status: "error", optimistic: !sent, updated_at: now() });
      upsertMessage(conversationId, { id: `error-${mutationId}`, conversation_id: conversationId, sequence: (sent?.sequence ?? optimistic.sequence) + 0.1, client_mutation_id: null, role: "assistant", content: "The response could not be generated. Use retry to send the message again.", images: [], reasoning_content: "", content_format: "markdown", status: "error", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now(), optimistic: true, retry_message_id: (sent ?? optimistic).id });
      setError(cause instanceof Error ? cause.message : "Message delivery failed.");
    } finally { setBusy(false); if (controller) endStream(); }
  }
  const handleEditMessage = useEvent(editMessage);
  const handleDeleteMessage = useEvent(deleteMessage);
  const handleRetryMessage = useEvent(retryMessage);
  /**
   * Streams one assistant turn over a user message: placeholder row, live tool and
   * reasoning events, then the server's final message. Shared by send and retry.
   */
  async function runTurn(conversationId: string, base: Message, options: GenerationSettings, search: boolean, pendingId: string, seed: ToolActivity[], signal?: AbortSignal) {
    let streamedContent = "";
    let streamedReasoning = "";
    let toolEvents = seed;
    const sequence = base.sequence + 0.5;
    const render = () => ({ ...base, id: pendingId, sequence, client_mutation_id: null, role: "assistant" as const, content: streamedContent, images: [] as string[], reasoning_content: streamedReasoning, status: "streaming" as const, model: null, token_count: 0, search_sources: [], tool_events: toolEvents });
    upsertMessage(conversationId, render());
    const onEvent = (event: StreamEvent) => {
      if (event.type === "tool") toolEvents = updateToolEvents(toolEvents, event.tool);
      else if (event.type === "reasoning") {
        const delta = event.delta ?? "";
        streamedReasoning += delta;
        const id = `reasoning-${event.round ?? 0}`;
        const previous = toolEvents.find((item) => item.id === id);
        toolEvents = updateToolEvents(toolEvents, { id, name: "reasoning", status: "completed", round: event.round, detail: `${previous?.detail ?? ""}${delta}` });
      } else streamedContent += event.delta ?? "";
      upsertMessage(conversationId, render());
    };
    const answer = options.stream ? await api.respondStream(conversationId, base.id, search, options, onEvent, signal) : await api.respond(conversationId, base.id, search, options);
    return { answer, toolEvents };
  }
  async function retryMessage(message: Message) {
    if (!activeId) return;
    const target = message.retry_message_id ? (useAppStore.getState().messages[activeId]?.find((item) => item.id === message.retry_message_id) ?? message) : message;
    if (message.retry_message_id) removeMessage(activeId, message.id);
    if (target.id.startsWith("local-")) { await sendMessage(activeId, target.content, searchEnabled, generation, target.images ?? []); return; }
    const pendingId = `retry-${target.id}`;
    let seed: ToolActivity[] = [];
    try { seed = (await api.agentRun(activeId, target.id)).events ?? []; } catch { /* no previous run */ }
    upsertMessage(activeId, { ...target, status: "pending", updated_at: now() });
    const controller = generation.stream ? beginStream() : null;
    setBusy(true);
    try {
      const turn = await runTurn(activeId, target, generation, searchEnabled, pendingId, seed, controller?.signal);
      removeMessage(activeId, pendingId);
      upsertMessage(activeId, { ...target, status: "complete", updated_at: now() });
      upsertMessage(activeId, { ...turn.answer, tool_events: turn.toolEvents });
    } catch (cause) {
      removeMessage(activeId, pendingId);
      if ((cause as Error)?.name === "AbortError") { upsertMessage(activeId, { ...target, status: "complete", updated_at: now() }); return; }
      upsertMessage(activeId, { ...target, status: "error", updated_at: now() });
      setError(cause instanceof Error ? cause.message : "Retry failed.");
    } finally { setBusy(false); if (controller) endStream(); }
  }
  async function editMessage(message: Message, content: string) {
    if (!activeId) return;
    const before = useAppStore.getState().messages[activeId] ?? [];
    upsertMessage(activeId, { ...message, content });
    try {
      upsertMessage(activeId, await api.updateMessage(activeId, message.id, content));
      const chats = await api.conversations();
      setConversations(chats.items);
      setConversationCursor(chats.next_cursor);
    }
    catch (cause) { setMessages(activeId, before); setError(cause instanceof Error ? cause.message : "Could not edit message."); }
  }
  async function deleteMessage(message: Message) {
    if (!activeId) return;
    const before = useAppStore.getState().messages[activeId] ?? [];
    removeMessage(activeId, message.id);
    if (message.optimistic || message.id.startsWith("local-") || message.id.startsWith("error-") || message.id.startsWith("assistant-") || message.id.startsWith("retry-")) return;
    try {
      await api.deleteMessage(activeId, message.id);
      const chats = await api.conversations();
      setConversations(chats.items);
      setConversationCursor(chats.next_cursor);
    }
    catch (cause) { setMessages(activeId, before); setError(cause instanceof Error ? cause.message : "Could not delete message."); }
  }

  const toggleFavorite = useCallback(async function toggleFavorite(id: string) {
    const before = useAppStore.getState().conversations;
    const item = before.find((conversation) => conversation.id === id);
    if (!item) return;
    setConversations(before.map((conversation) => conversation.id === id ? { ...conversation, is_favorite: !conversation.is_favorite } : conversation));
    try { const updated = await api.updateConversation(id, { is_favorite: !item.is_favorite }); setConversations(useAppStore.getState().conversations.map((conversation) => conversation.id === id ? updated : conversation)); }
    catch (cause) { setConversations(before); setError(cause instanceof Error ? cause.message : "Unable to update favorite."); }
  }, [setConversations, setError]);
  async function compactConversation() {
    if (!activeId || compacting) return;
    setCompacting(true);
    try {
      const result = await api.compact(activeId);
      upsertMessage(activeId, result.message);
      const chats = await api.conversations();
      setConversations(chats.items);
      setConversationCursor(chats.next_cursor);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Compaction failed."); } finally { setCompacting(false); }
  }

  const handleLookup = useCallback((word: string, anchor: { x: number; y: number }) => setLookup({ word, ...anchor }), []);
  return <main className={`app-shell ${theme} ${sidebarCollapsed ? "sidebar-is-collapsed" : ""}`}><Sidebar conversations={conversations} activeId={activeId} open={sidebarOpen} collapsed={sidebarCollapsed} onNew={() => void newChat()} onSelect={setActiveId} onRename={async (id, title) => { try { const updated = await api.updateConversation(id, { title }); setConversations(useAppStore.getState().conversations.map((item) => item.id === id ? updated : item)); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to rename conversation."); } }} hasMore={conversationCursor !== null} loadingMore={loadingConversations} onLoadMore={loadMoreConversations} onToggleFavorite={toggleFavorite} onSettings={() => setProviderOpen(true)} onClose={() => setSidebarOpen(false)} onCollapse={() => setSidebarCollapsed(true)} userName={session?.user.display_name ?? "Account"} onSignOut={signOut} />{sidebarOpen && <div className="sidebar-backdrop" onClick={() => setSidebarOpen(false)} />}<section className="chat-shell"><header className="chat-header"><button className="icon-button desktop-reopen" aria-label="Open navigation" onClick={() => setSidebarCollapsed(false)}><PanelLeftOpen size={19} /></button><button className="icon-button mobile-only" aria-label="Open navigation" onClick={() => setSidebarOpen(true)}><Menu size={20} /></button><div className="header-title"><span>{activeConversation?.title ?? "malim_chat"}</span></div>{activeConversation && <ModelSelector conversation={activeConversation} providers={providers} open={modelMenuOpen} busy={modelChanging} onToggle={() => setModelMenuOpen(!modelMenuOpen)} onChange={changeModel} />}</header>{error && <div className="notice"><CircleAlert size={16} /><span>{error}</span><button className="icon-button" aria-label="Dismiss" onClick={() => setError(null)}><X size={16} /></button></div>}<ConversationView conversation={activeConversation} messages={activeId ? messages[activeId] ?? [] : []} hasOlder={Boolean(activeId && messageCursors[activeId])} loadingOlder={loadingOlder} onLoadOlder={loadOlderMessages} providers={providers} searchEnabled={searchEnabled} busy={busy} compacting={compacting} theme={theme} lookupActive={lookup !== null} streaming={streaming} onStop={stopStreaming} onToggleSearch={() => setSearchEnabled(!searchEnabled)} generation={generation} onGenerationChange={changeGeneration} onSend={(content, images) => activeId ? sendMessage(activeId, content, searchEnabled, generation, images ?? []) : Promise.resolve()} onEdit={handleEditMessage} onDelete={handleDeleteMessage} onRetry={handleRetryMessage} onLookup={handleLookup} onCompact={compactConversation} /><RightNavigator messages={activeId ? messages[activeId] ?? [] : []} /></section>{providerOpen && <ProviderDialog providers={providers} onClose={() => setProviderOpen(false)} onChanged={async () => { setProviders(await api.providers()); }} />}{lookup && <DictionaryPopover word={lookup.word} anchor={lookup} onClose={() => { clearLookupHighlight(); setLookup(null); }} />}</main>;
}

function useFlip(ref: { current: HTMLElement | null }, deps: unknown[]) {
  const previous = useRef<Map<string, DOMRect>>(new Map());
  useLayoutEffect(() => {
    const container = ref.current;
    if (!container) return;
    const current = new Map<string, DOMRect>();
    container.querySelectorAll<HTMLElement>("[data-flip-id]").forEach((node) => { current.set(node.dataset.flipId ?? "", node.getBoundingClientRect()); });
    const moved: HTMLElement[] = [];
    for (const [id, rect] of previous.current) {
      const node = container.querySelector<HTMLElement>(`[data-flip-id="${CSS.escape(id)}"]`);
      const now = node ? current.get(id) : undefined;
      if (node && now && (Math.abs(now.top - rect.top) > 1 || Math.abs(now.left - rect.left) > 1)) {
        node.style.transition = "none";
        node.style.transform = `translate(${rect.left - now.left}px, ${rect.top - now.top}px)`;
        moved.push(node);
      }
    }
    previous.current = current;
    if (!moved.length) return;
    requestAnimationFrame(() => { moved.forEach((node) => { node.style.transition = "transform 300ms cubic-bezier(.2,.7,.2,1)"; node.style.transform = ""; }); const clear = (event: TransitionEvent) => { const target = event.target as HTMLElement; target.style.transition = ""; target.style.transform = ""; target.removeEventListener("transitionend", clear); }; moved.forEach((node) => node.addEventListener("transitionend", clear)); });
    // The caller forwards its own dependency list, so the rule cannot check it statically.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
function ConversationRow({ conversation, active, onSelect, onRename, onToggleFavorite }: { conversation: Conversation; active: boolean; onSelect: (id: string) => void; onRename: (id: string, title: string) => Promise<void>; onToggleFavorite: (id: string) => void }) {
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(conversation.title);
  async function finishRename() {
    const title = draft.trim();
    if (title) await onRename(conversation.id, title);
    setRenaming(false);
  }
  return <div className={`conversation-row ${active ? "selected" : ""}`} data-flip-id={conversation.id}>{renaming ? <form className="conversation-rename" onSubmit={(event) => { event.preventDefault(); void finishRename(); }}><input autoFocus value={draft} maxLength={160} onChange={(event) => setDraft(event.target.value)} onBlur={() => void finishRename()} /></form> : <button type="button" onClick={() => onSelect(conversation.id)}><MessageSquare size={16} /><span>{conversation.title}</span></button>}<button type="button" className="row-action" aria-label={`Rename ${conversation.title}`} title="Rename conversation" onMouseDown={(event) => event.preventDefault()} onClick={() => { setDraft(conversation.title); setRenaming(true); }}><Edit3 size={15} /></button><button type="button" className={`row-action star ${conversation.is_favorite ? "active" : ""}`} aria-label={conversation.is_favorite ? "Remove from favorites" : "Add to favorites"} title={conversation.is_favorite ? "Remove from favorites" : "Add to favorites"} onMouseDown={(event) => event.preventDefault()} onClick={() => onToggleFavorite(conversation.id)}><Star size={15} fill={conversation.is_favorite ? "currentColor" : "none" } /></button></div>;
}
const MemoConversationRow = memo(ConversationRow, (previous, next) => previous.conversation === next.conversation && previous.active === next.active);

function Sidebar({ conversations, activeId, open, collapsed, hasMore, loadingMore, onLoadMore, onNew, onSelect, onRename, onToggleFavorite, onSettings, onClose, onCollapse, userName, onSignOut }: { conversations: Conversation[]; activeId: string | null; open: boolean; collapsed: boolean; hasMore: boolean; loadingMore: boolean; onLoadMore: () => Promise<void>; onNew: () => void; onSelect: (id: string) => void; onRename: (id: string, title: string) => Promise<void>; onToggleFavorite: (id: string) => void; onSettings: () => void; onClose: () => void; onCollapse: () => void; userName: string; onSignOut: () => void }) {
  const [favoritesCollapsed, setFavoritesCollapsed] = useState(() => localStorage.getItem("malim-favorites-collapsed") === "1");
  const historyRef = useRef<HTMLDivElement | null>(null);
  const loadMoreRef = useRef<HTMLDivElement | null>(null);
  const favorites = useMemo(() => conversations.filter((conversation) => conversation.is_favorite), [conversations]);
  const recent = useMemo(() => conversations.filter((conversation) => !conversation.is_favorite), [conversations]);
  useEffect(() => { localStorage.setItem("malim-favorites-collapsed", favoritesCollapsed ? "1" : "0"); }, [favoritesCollapsed]);
  const conversationOrder = useMemo(() => conversations.map((conversation) => conversation.id).join(","), [conversations]);
  useFlip(historyRef, [conversationOrder]);
  useEffect(() => {
    const node = loadMoreRef.current;
    if (!node || !hasMore || loadingMore) return;
    const observer = new IntersectionObserver((entries) => { if (entries.some((entry) => entry.isIntersecting)) void onLoadMore(); }, { root: historyRef.current, rootMargin: "240px" });
    observer.observe(node);
    return () => observer.disconnect();
  }, [favorites.length, hasMore, loadingMore, onLoadMore, recent.length]);
  const row = (conversation: Conversation) => <MemoConversationRow key={conversation.id} conversation={conversation} active={conversation.id === activeId} onSelect={onSelect} onRename={onRename} onToggleFavorite={onToggleFavorite} />;
  return <aside className={`sidebar ${open ? "open" : ""} ${collapsed ? "collapsed" : ""}`}><div className="sidebar-top"><div className="wordmark"><Bot size={20} /><span>malim_chat</span></div><button className="icon-button desktop-only" aria-label="Collapse navigation" onClick={onCollapse}><PanelLeftClose size={19} /></button><button className="icon-button mobile-only" aria-label="Close navigation" onClick={onClose}><X size={19} /></button></div><button className="new-chat" type="button" onClick={onNew}><SquarePen size={17} />New chat</button><nav aria-label="Conversation history" className="history" ref={historyRef}>{favorites.length > 0 && <><div className="favorites-head"><button type="button" className="section-toggle" aria-expanded={!favoritesCollapsed} onClick={() => setFavoritesCollapsed(!favoritesCollapsed)}><ChevronDown size={14} className={favoritesCollapsed ? "collapsed" : ""} /><span>Starred</span><span className="section-count">{favorites.length}</span></button></div><div className={`favorites-body ${favoritesCollapsed ? "collapsed" : ""}`}><div className="favorites-inner">{favorites.map(row)}</div></div></>}<p>{favorites.length > 0 ? "Recent" : "Conversations"}</p>{recent.map(row)}<div ref={loadMoreRef} className="sidebar-loader">{loadingMore && <LoaderCircle className="spin" size={15} />}</div></nav><div className="sidebar-bottom"><button className="settings-button" type="button" onClick={onSettings}><Settings2 size={17} />Providers & settings</button><div className="account-row"><div className="avatar">{userName.slice(0, 1).toUpperCase()}</div><span>{userName}</span><button type="button" className="row-action" aria-label="Sign out" title="Sign out" onClick={onSignOut}><LogOut size={17} /></button></div></div></aside>;
}

function ModelSelector({ conversation, providers, open, busy, onToggle, onChange }: { conversation: Conversation; providers: Provider[]; open: boolean; busy: boolean; onToggle: () => void; onChange: (providerId: string, model: string) => Promise<void> }) {
  const activeProvider = providers.find((provider) => provider.id === conversation.model_provider_id);
  const [providerId, setProviderId] = useState(conversation.model_provider_id ?? providers[0]?.id ?? "");
  const [model, setModel] = useState(conversation.model ?? activeProvider?.models[0]?.model ?? "");
  useEffect(() => { setProviderId(conversation.model_provider_id ?? providers[0]?.id ?? ""); setModel(conversation.model ?? activeProvider?.models[0]?.model ?? ""); }, [conversation.id, conversation.model_provider_id, conversation.model, activeProvider?.models, providers]);
  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const groups = (selectedProvider?.models ?? []).reduce<Record<string, ProviderModel[]>>((all, item) => { (all[item.group_name] ??= []).push(item); return all; }, {});
  return <div className="model-selector"><button type="button" className="model-button" aria-expanded={open} disabled={busy} onClick={onToggle}><span>{model || "Choose model"}</span>{busy ? <LoaderCircle className="spin" size={15} /> : <ChevronsUpDown size={15} />}</button>{open && <div className="model-popover"><div className="model-picker-heading">Provider</div><div className="provider-options">{providers.map((provider) => <button type="button" key={provider.id} disabled={busy} className={provider.id === providerId ? "selected" : ""} onClick={() => { setProviderId(provider.id); setModel(provider.models[0]?.model ?? ""); }}>{provider.name}</button>)}</div><div className="model-picker-heading">Model</div><div className="configured-models">{Object.entries(groups).map(([group, items]) => <section key={group}><h4>{group}</h4>{items.map((item) => <button type="button" key={item.id} disabled={busy} className={item.model === model ? "selected" : ""} onClick={() => { setModel(item.model); void onChange(providerId, item.model); }}>{item.model}</button>)}</section>)}</div></div>}</div>;
}

function ConversationView({ conversation, messages, hasOlder, loadingOlder, onLoadOlder, providers, searchEnabled, busy, compacting, theme, lookupActive, streaming, onStop, generation, onGenerationChange, onToggleSearch, onSend, onEdit, onDelete, onRetry, onLookup, onCompact }: { conversation: Conversation | null; messages: Message[]; hasOlder: boolean; loadingOlder: boolean; onLoadOlder: (id: string) => Promise<void>; providers: Provider[]; searchEnabled: boolean; busy: boolean; compacting: boolean; theme: "light" | "dark"; lookupActive: boolean; streaming: boolean; onStop: () => void; generation: GenerationSettings; onGenerationChange: (value: GenerationSettings) => void; onToggleSearch: () => void; onSend: (content: string, images?: string[]) => Promise<void>; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void; onCompact: () => Promise<void> }) {
  const [input, setInput] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  const [imageError, setImageError] = useState<string | null>(null);
  const messageListRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const conversationIdRef = useRef<string | null>(null);
  const initialContentLoadedRef = useRef(false);
  const wasStreamingRef = useRef(false);
  const atBottomRef = useRef(true);
  const followResponseRef = useRef(true);
  const isStreaming = useMemo(() => messages.some((message) => message.status === "streaming"), [messages]);
  useEffect(() => {
    const node = messageListRef.current;
    if (!node) return;
    const updatePosition = () => {
      const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight;
      atBottomRef.current = distanceFromBottom <= 24;
      if (isStreaming) followResponseRef.current = atBottomRef.current;
    };
    updatePosition();
    node.addEventListener("scroll", updatePosition, { passive: true });
    return () => node.removeEventListener("scroll", updatePosition);
  }, [conversation?.id, isStreaming]);
  // Keep streamed responses pinned only while the reader remains at the bottom.
  useLayoutEffect(() => {
    const node = messageListRef.current;
    if (!node) return;
    if (conversationIdRef.current !== conversation?.id) {
      conversationIdRef.current = conversation?.id ?? null;
      initialContentLoadedRef.current = false;
      wasStreamingRef.current = false;
      atBottomRef.current = true;
      followResponseRef.current = true;
    }
    if (!initialContentLoadedRef.current && messages.length > 0) {
      node.scrollTop = node.scrollHeight;
      initialContentLoadedRef.current = true;
      atBottomRef.current = true;
      followResponseRef.current = true;
    }
    if (isStreaming) {
      if (!wasStreamingRef.current) followResponseRef.current = atBottomRef.current;
      if (followResponseRef.current) {
        node.scrollTop = node.scrollHeight;
        atBottomRef.current = true;
      }
    }
    wasStreamingRef.current = isStreaming;
  }, [conversation?.id, isStreaming, messages]);
  useEffect(() => { const node = inputRef.current; if (!node) return; node.style.height = "auto"; node.style.height = `${Math.min(node.scrollHeight, 180)}px`; }, [input]);
  useEffect(() => { setImages([]); setImageError(null); setDragging(false); }, [conversation?.id]);
  if (!conversation) return <section className="empty-state"><div className="empty-logo"><Bot size={34} /></div><h1>How can I help you today?</h1><p>Start a new conversation to work with your configured AI providers.</p></section>;
  const usage = Math.min(100, Math.round((conversation.context_tokens / conversation.context_window) * 100));
  const hasActiveProvider = providers.some((provider) => provider.id === conversation.model_provider_id && provider.models.some((item) => item.model === conversation.model));
  const activeModel = providers.find((item) => item.id === conversation.model_provider_id)?.models.find((item) => item.model === conversation.model);
  // Both hosted dialects search on the provider's side, which replaces our ReAct loop and
  // with it the composer's Web-search switch.
  const nativeSearch = activeModel?.kind === "openai_responses" || (activeModel?.kind === "anthropic" && activeModel.hosted_tools === true);
  function addFiles(list: FileList | File[] | null) {
    if (!list) return;
    for (const file of Array.from(list)) {
      if (!file.type.startsWith("image/")) { setImageError("Only image files can be attached."); continue; }
      if (file.size > 5 * 1024 * 1024) { setImageError("Each image must be 5 MB or smaller."); continue; }
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result !== "string") return;
        setImages((current) => {
          if (current.length >= 8) { setImageError("A message can hold up to 8 images."); return current; }
          setImageError(null);
          return [...current, reader.result as string];
        });
      };
      reader.readAsDataURL(file);
    }
  }
  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const files: File[] = [];
    for (const item of Array.from(event.clipboardData?.items ?? [])) {
      const file = item.kind === "file" && item.type.startsWith("image/") ? item.getAsFile() : null;
      if (file) files.push(file);
    }
    if (files.length > 0) { event.preventDefault(); addFiles(files); }
  }
  function send() {
    const text = input.trim();
    if ((!text && images.length === 0) || busy || !hasActiveProvider) return;
    const payload = images;
    setInput("");
    setImages([]);
    setImageError(null);
    void onSend(text, payload);
  }
  function submit(event: FormEvent) { event.preventDefault(); send(); }
  return <><section ref={messageListRef} className={generation.stream ? "message-list stream-output" : "message-list"}>{hasOlder && <button className="load-older" type="button" disabled={loadingOlder} onClick={() => conversation && void onLoadOlder(conversation.id)}>{loadingOlder ? "Loading older..." : "Load older messages"}</button>}{messages.map((message, index) => <MemoMessageBubble key={message.id} message={message} markdownEnabled={generation.enable_markdown} theme={theme} isLast={index === messages.length - 1} lookupActive={lookupActive} onEdit={onEdit} onDelete={onDelete} onRetry={onRetry} onLookup={onLookup} />)}</section><footer className="composer-wrap"><div className="context-meter"><span>Context {conversation.context_tokens.toLocaleString()} / {conversation.context_window.toLocaleString()} tokens</span><div><i style={{ width: `${usage}%` }} /></div><button type="button" disabled={compacting} onClick={() => void onCompact()} title="Compact previous context">{compacting ? <><LoaderCircle className="spin" size={12} />Compacting</> : "Compact"}</button></div><form className={`composer ${dragging ? "dragging" : ""}`} onSubmit={submit} onDragOver={(event) => { event.preventDefault(); setDragging(true); }} onDragLeave={() => setDragging(false)} onDrop={(event) => { event.preventDefault(); setDragging(false); addFiles(event.dataTransfer.files); }}><input ref={fileInputRef} type="file" accept="image/*" multiple hidden onChange={(event) => { addFiles(event.target.files); event.target.value = ""; }} />{images.length > 0 && <div className="image-previews">{images.map((src, index) => <div className="image-preview" key={`${index}-${src.slice(0, 40)}`}><img src={src} alt={`Attachment ${index + 1}`} /><button type="button" aria-label={`Remove image ${index + 1}`} title="Remove image" onClick={() => setImages(images.filter((_, item) => item !== index))}><X size={13} /></button></div>)}</div>}{imageError && <p className="image-error">{imageError}</p>}<textarea ref={inputRef} value={input} onChange={(event) => setInput(event.target.value)} placeholder={hasActiveProvider ? "Message malim_chat" : "Choose a model from the conversation header"} disabled={!hasActiveProvider || busy || compacting} rows={1} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); send(); } }} onPaste={handlePaste} /><div className="composer-controls"><button type="button" className="attach-button" aria-label="Attach images" title="Attach images" onClick={() => fileInputRef.current?.click()} disabled={!hasActiveProvider || busy || compacting}><ImagePlus size={17} /></button>{!nativeSearch && <button type="button" className={searchEnabled ? "search-chip enabled" : "search-chip"} aria-pressed={searchEnabled} onClick={onToggleSearch}><Search size={14} />{searchEnabled ? "Web search on" : "Web search off"}</button>}<GenerationButton generation={generation} onChange={onGenerationChange} kind={activeModel?.kind} model={conversation.model ?? ""} hosted={nativeSearch} /><button type={streaming ? "button" : "submit"} className={`send-button${streaming ? " stop" : ""}`} aria-label={streaming ? "Stop generating" : "Send message"} title={streaming ? "Stop generating" : "Send"} onClick={streaming ? (event) => { event.preventDefault(); onStop(); } : undefined} disabled={!streaming && ((!input.trim() && images.length === 0) || busy || compacting || !hasActiveProvider)}>{streaming ? <Square size={17} /> : busy ? <LoaderCircle className="spin" size={18} /> : <Check size={18} />}</button></div></form><p className="disclaimer">AI responses may be inaccurate. Review important information.</p></footer></>;
}

function RightNavigator({ messages }: { messages: Message[] }) {
  const innerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const questions = useMemo(() => messages.filter((message) => message.role === "user"), [messages]);
  const rowHeight = 34;
  useEffect(() => {
    const measure = () => setViewportHeight(innerRef.current?.clientHeight ?? 0);
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [questions.length]);
  const overscan = 20;
  const start = viewportHeight ? Math.max(0, Math.floor(scrollTop / rowHeight) - overscan) : 0;
  const visibleCount = viewportHeight ? Math.ceil(viewportHeight / rowHeight) + overscan * 2 : Math.min(80, questions.length);
  const visibleQuestions = questions.slice(start, start + visibleCount);
  return <aside className="right-navigator" aria-label="Conversation navigation"><div className="right-nav-inner" ref={innerRef} onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}><strong>In this chat</strong>{questions.length === 0 ? <span className="muted">No questions yet</span> : <><div style={{ height: start * rowHeight }} />{visibleQuestions.map((message) => <button key={message.id} onClick={() => document.getElementById(`message-${message.id}`)?.scrollIntoView({ behavior: "smooth", block: "center" })}><span className="right-nav-label">{message.content || "[Image]"}</span></button>)}<div style={{ height: Math.max(0, (questions.length - start - visibleQuestions.length) * rowHeight) }} /></>}</div></aside>;
}

function Markdown({ content, theme }: { content: string; theme: "light" | "dark" }) {
  return <ReactMarkdown remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex]} components={{ code({ className, children, ...props }) { const language = /language-(\w+)/.exec(className ?? "")?.[1]; const source = String(children).replace(/\n$/, ""); return language ? <CodeBlock language={language} code={source} theme={theme} /> : <code className={className} {...props}>{children}</code>; } }}>{preprocessLatex(content)}</ReactMarkdown>;
}
const MemoMarkdown = memo(Markdown);
const MemoMessageBubble = memo(MessageBubble);

function GenerationButton({ generation, onChange, kind, model, hosted }: { generation: GenerationSettings; onChange: (value: GenerationSettings) => void; kind?: ProviderKind; model: string; hosted: boolean }) {
  const [open, setOpen] = useState(false);
  const controlRef = useRef<HTMLDivElement>(null);
  // Responses stores the conversation on its own side, so there is no window to trim; and
  // wherever the provider runs its own tools there is no ReAct loop of ours to bound.
  const chained = kind === "openai_responses";
  const supportsReasoning = (kind === "openai_compatible" || kind === "openai_responses") && /(?:gpt-5|\bo[1-9]\b|codex|reasoner|thinking|deepseek-r1)/i.test(model);
  const effort = ["low", "medium", "high"] as const;
  useEffect(() => { const outside = (event: PointerEvent) => { if (!controlRef.current?.contains(event.target as Node)) setOpen(false); }; document.addEventListener("pointerdown", outside, true); return () => document.removeEventListener("pointerdown", outside, true); }, []);
  const context = generation.context_rounds ?? null; const tools = generation.tool_rounds ?? null;
  return <div ref={controlRef} className="generation-control"><button type="button" className="search-chip" onClick={() => setOpen(!open)} title="Generation parameters"><Settings2 size={14} />Parameters</button>{open && <div className="generation-popover"><label><span>Temperature <b>{generation.temperature.toFixed(1)}</b></span><input type="range" min="0" max="2" step="0.1" value={generation.temperature} onChange={(event) => onChange({ ...generation, temperature: Number(event.target.value) })} /></label>{supportsReasoning && <label><span>Reasoning effort <b>{generation.reasoning_effort}</b></span><input type="range" min="0" max="2" step="1" value={effort.indexOf(generation.reasoning_effort as typeof effort[number])} onChange={(event) => onChange({ ...generation, reasoning_effort: effort[Number(event.target.value)] })} /><small>Low <i /> Medium <i /> High</small></label>}{!chained && <label><span>Recent context rounds <b>{context === null ? "All" : context}</b></span><input type="range" min="0" max="20" step="1" value={context === null ? 20 : context} onChange={(event) => { const value = Number(event.target.value); onChange({ ...generation, context_rounds: value >= 20 ? null : value }); }} /><small>0 <i /> All</small></label>}{!hosted && <label><span>Maximum ReAct rounds <b>{tools === null ? "Unlimited" : tools}</b></span><input type="range" min="1" max="20" step="1" value={tools === null ? 20 : tools} onChange={(event) => { const value = Number(event.target.value); onChange({ ...generation, tool_rounds: value >= 20 ? null : value }); }} /><small>1 <i /> Unlimited</small></label>}{hosted && <p className="hint">The provider runs its own search, page reading and code execution, so there is no ReAct round limit to set.</p>}<label className="markdown-toggle"><span>Enable Markdown</span><input type="checkbox" checked={generation.enable_markdown} onChange={(event) => onChange({ ...generation, enable_markdown: event.target.checked })} /></label><label className="markdown-toggle"><span>Stream output</span><input type="checkbox" checked={generation.stream} onChange={(event) => onChange({ ...generation, stream: event.target.checked })} /></label></div>}</div>;
}

function preprocessLatex(input: string): string {
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

function applyLookupHighlight(node: Node, start: number, end: number) {
  try {
    const store = (CSS as unknown as { highlights?: { set(name: string, highlight: unknown): void; delete(name: string): void } }).highlights;
    const HighlightCtor = (window as unknown as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
    if (!store || !HighlightCtor) return;
    const range = new Range();
    range.setStart(node, start);
    range.setEnd(node, end);
    store.set("dictionary-lookup", new HighlightCtor(range));
  } catch { /* highlight API unavailable */ }
}
function clearLookupHighlight() {
  try { (CSS as unknown as { highlights?: { delete(name: string): void } }).highlights?.delete("dictionary-lookup"); } catch { /* noop */ }
}
function CodeBlock({ language, code, theme }: { language: string; code: string; theme: "light" | "dark" }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await copyText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch { /* clipboard unavailable */ }
  }
  return <div className="code-block"><button type="button" className={copied ? "code-copy copied" : "code-copy"} title={copied ? "Copied" : "Copy code"} onClick={() => void copy()}>{copied ? <Check size={13} /> : <Copy size={13} />}</button><SyntaxHighlighter language={language} style={theme === "dark" ? oneDark : oneLight} showLineNumbers wrapLongLines customStyle={{ margin: "0", background: "transparent", padding: "12px 0" }}>{code}</SyntaxHighlighter></div>;
}

function sourceHost(url: string) {
  try { return new URL(url).hostname.replace(/^www\./, ""); }
  catch { return url; }
}

function SourceList({ sources }: { sources: Message["search_sources"] }) {
  if (sources.length === 0) return null;
  const queries = [...new Set(sources.map((source) => source.query).filter((query): query is string => Boolean(query)))];
  return <details className="sources"><summary><Search size={14} />{sources.length} web source{sources.length === 1 ? "" : "s"}</summary>{queries.length > 0 && <p className="source-query">Search: {queries.join(" · ")}</p>}<div className="source-list">{sources.map((source, index) => <a href={source.url} target="_blank" rel="noreferrer" key={`${source.url}-${index}`}><span className="source-title">{source.title || sourceHost(source.url)}</span><span className="source-meta">{sourceHost(source.url)} · {source.engine || "SearXNG"}</span>{source.content && <span className="source-snippet">{source.content}</span>}</a>)}</div></details>;
}

function ToolActivityList({ events, reasoning, streaming, theme, markdown }: { events?: ToolActivity[]; reasoning?: string; streaming?: boolean; theme: "light" | "dark"; markdown: boolean }) {
  const [open, setOpen] = useState(Boolean(streaming));
  const [height, setHeight] = useState(0);
  const contentRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (!streaming) setOpen(false); }, [streaming]);
  useEffect(() => { const node = contentRef.current; if (!node) return; const measure = () => setHeight(node.scrollHeight); measure(); const observer = new ResizeObserver(measure); observer.observe(node); return () => observer.disconnect(); }, [events, reasoning]);
  // Checkpoints saved before reasoning became a tool event carry bare `{delta, round}` rows.
  const steps = (events ?? []).flatMap((event, index) => {
    if (event.name) return [event];
    const delta = (event as ToolActivity & { delta?: string }).delta;
    if (!delta) return [];
    return [{ id: `reasoning-${event.round ?? index}`, name: "reasoning", status: "completed", round: event.round, detail: delta } satisfies ToolActivity];
  });
  if (!steps.length && !reasoning) return null;
  function label(event: ToolActivity) {
    const query = event.input?.query;
    const host = event.input?.url ? sourceHost(event.input.url) : "";
    if (event.status === "failed") return event.detail || `${event.name.replace(/_/g, " ")} failed`;
    if (event.name === "planning") return event.status === "running" ? "Planning research" : "Research step planned";
    if (event.name === "web_search") {
      if (event.status === "running") return query ? `Searching: ${query}` : "Searching the web";
      return event.source_count !== undefined ? `Search complete: ${event.source_count} sources` : "Search complete";
    }
    if (event.name === "open_web_page") return event.status === "running" ? `Reading: ${host || "source"}` : `Read: ${host || "source"}`;
    // The provider's own fetch and sandbox: their calls arrive with the result already
    // attached, so there is no input of ours to show.
    if (event.name === "web_fetch") return event.status === "running" ? "Reading a page" : `Read: ${sourceHost(event.detail ?? "") || "source"}`;
    if (event.name === "code_execution") return event.status === "running" ? "Running code" : event.detail ? `Code output: ${event.detail}` : "Code finished";
    if (event.name === "drafting") return event.status === "running" ? "Drafting answer" : "Answer drafted";
    if (event.name === "reasoning") return event.detail || "Reasoning";
    return event.status === "running" ? `${event.name.replace(/_/g, " ")} in progress` : `${event.name.replace(/_/g, " ")} complete`;
  }
  function copy(event: ToolActivity) {
    const text = label(event);
    if (event.name !== "reasoning" || !markdown) return <span className="tool-step-copy">{text}</span>;
    return <div className="tool-step-copy markdown-content"><Markdown content={text} theme={theme} /></div>;
  }
  return <section className={`agent-timeline ${open ? "open" : "collapsed"}`} aria-label="Agent process" aria-live="polite"><button type="button" className="agent-timeline-toggle" aria-expanded={open} onClick={() => setOpen(!open)}><span>{streaming ? "Agent working" : "Agent process"}</span><small>{open ? "Collapse" : "Expand"}</small></button><div className="agent-timeline-collapse" style={{ maxHeight: open ? height : 0 }}><div ref={contentRef} className="tool-activity">{steps.map((event) => <div key={event.id} className={`tool-step ${event.status}`}><span className="tool-step-icon">{event.name === "reasoning" ? <Bot size={12} /> : event.status === "running" ? <LoaderCircle className="spin" size={13} /> : event.status === "failed" ? <CircleAlert size={13} /> : <Check size={13} />}</span>{copy(event)}</div>)}{!steps.some((event) => event.name === "reasoning") && reasoning && <div className="tool-step completed"><span className="tool-step-icon"><Bot size={12} /></span>{markdown ? <div className="tool-step-copy markdown-content"><Markdown content={reasoning} theme={theme} /></div> : <span className="tool-step-copy">{reasoning}</span>}</div>}</div></div></section>;
}

function MessageBubble({ message, markdownEnabled, theme, isLast, lookupActive, onEdit, onDelete, onRetry, onLookup }: { message: Message; markdownEnabled: boolean; theme: "light" | "dark"; isLast: boolean; lookupActive: boolean; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void }) {
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [copied, setCopied] = useState(false);
  useEffect(() => { setDraft(message.content); }, [message.content]);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const selectionCaptured = useRef(false);
  const lookupActiveRef = useRef(lookupActive);
  useEffect(() => { lookupActiveRef.current = lookupActive; }, [lookupActive]);
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
    const isWord = (char?: string) => char ? /[A-Za-z\u00C0-\u024F\u0370-\u052F'\u2019]/.test(char) : false;
    const offset = range.startOffset;
    if (!isWord(text[offset - 1]) && !isWord(text[offset])) return;
    let start = offset;
    while (start > 0 && isWord(text[start - 1])) start--;
    let end = offset;
    while (end < text.length && isWord(text[end])) end++;
    const word = text.slice(start, end);
    if (word && word.length <= 120) { selectionCaptured.current = true; applyLookupHighlight(node, start, end); onLookup(word, { x: Math.min(window.innerWidth - 18, Math.max(18, x)), y: Math.min(window.innerHeight - 18, Math.max(18, y + 12)) }); }
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
        captureSelection();
      }
    }, 200);
  });
  useEffect(() => { document.addEventListener("selectionchange", onSelectionChange); return () => { document.removeEventListener("selectionchange", onSelectionChange); if (selectionTimer.current !== null) window.clearTimeout(selectionTimer.current); }; }, [onSelectionChange]);
  async function copyContent() {
    try {
      await copyText(content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch { /* clipboard unavailable */ }
  }
  function captureSelection() { if (lookupActiveRef.current) return; const selected = window.getSelection()?.toString().trim() ?? ""; if (selected && selected.length <= 120) { selectionCaptured.current = true; const range = window.getSelection()?.rangeCount ? window.getSelection()?.getRangeAt(0).getBoundingClientRect() : undefined; onLookup(selected, { x: Math.min(window.innerWidth - 18, Math.max(18, range?.left ?? window.innerWidth / 2)), y: Math.min(window.innerHeight - 18, Math.max(18, range?.bottom ?? window.innerHeight / 2)) }); } }
  const legacy = splitLegacyThinking(message.content);
  const content = legacy.content;
  const reasoning = [message.reasoning_content, legacy.reasoning].filter(Boolean).join("\n");
  const renderMarkdown = markdownEnabled && message.role === "assistant" && message.content_format === "markdown";
  return <article id={`message-${message.id}`} className={`message ${message.role === "user" ? "user" : message.role === "summary" ? "summary" : "assistant"} ${message.status === "error" ? "message-error" : ""}`}><div className="message-avatar">{message.role === "user" ? "You" : <Bot size={17} />}</div><div className="message-body" ref={bodyRef} onMouseUp={captureSelection} onTouchStart={handleTouchStart} onTouchMove={handleTouchMove} onTouchEnd={handleTouchEnd} onTouchCancel={cancelLongPress}>{editing ? <div className="edit-box"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} /><button className="primary small" type="button" onClick={() => { void onEdit(message, draft); setEditing(false); }}>Save</button><button className="text-button small" type="button" onClick={() => { setDraft(message.content); setEditing(false); }}>Cancel</button></div> : <><ToolActivityList events={message.tool_events} reasoning={reasoning} streaming={message.status === "streaming"} theme={theme} markdown={markdownEnabled} />{message.images?.length > 0 && <div className="message-images">{message.images.map((src) => <img key={src} src={src} alt="Attached image" />)}</div>}{message.status === "streaming" && !content && !reasoning ? <span className="typing"><i /><i /><i /></span> : (content || message.images?.length > 0) && <div className={`message-content ${renderMarkdown ? "markdown-content" : "plain-content"}`}>{renderMarkdown ? <MemoMarkdown content={content} theme={theme} /> : content}</div>}</>} {isLast && message.status === "error" && (message.role === "user" || message.retry_message_id) && <button type="button" className="retry-button" onClick={() => void onRetry(message)}>Retry response</button>}<SourceList sources={message.search_sources ?? []} /><div className="message-tools"><button type="button" className={copied ? "copied" : ""} title={copied ? "Copied" : "Copy"} onClick={() => void copyContent()}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>{!message.optimistic && message.role !== "system" && <button type="button" title="Edit" onClick={() => setEditing(true)}><Edit3 size={14} /></button>}<button type="button" className={confirmDelete ? "delete armed" : "delete"} title={confirmDelete ? "Click again to delete" : "Delete"} aria-label={confirmDelete ? "Confirm delete message" : "Delete message"} onBlur={() => setConfirmDelete(false)} onClick={(event) => { if (!confirmDelete) { event.preventDefault(); event.currentTarget.focus(); setConfirmDelete(true); return; } event.preventDefault(); setConfirmDelete(false); void onDelete(message); }}><Trash2 size={14} /></button>{message.model && message.status !== "streaming" && message.status !== "pending" && <span className="message-model">{message.model}</span>}</div></div></article>;
}

function splitLegacyThinking(value: string) {
  const reasoning: string[] = [];
  const content = value.replace(/<(think|thinking)>([\s\S]*?)(?:<\/\1>|$)/gi, (_, _tag, thought: string) => {
    if (thought.trim()) reasoning.push(thought.trim());
    return "";
  });
  return { content: content.trimStart(), reasoning: reasoning.join("\n") };
}

function ProviderDialog({ providers, onClose, onChanged: publish, }: { providers: Provider[]; onClose: () => void; onChanged: () => Promise<void> }) {
  // Bumping the revision remounts the settings form so it re-seeds from saved values.
  const [revision, setRevision] = useState(0);
  const onChanged = async () => { await publish(); setRevision((value) => value + 1); };
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ProviderKind>("openai_compatible");
  const [baseUrl, setBaseUrl] = useState("https://api.openai.com/v1");
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(providers[0]?.id ?? null);
  function setProviderKind(next: ProviderKind) {
    setKind(next);
    if (next === "anthropic") setBaseUrl("https://api.anthropic.com");
    else if (next === "openai_responses") setBaseUrl("https://api.openai.com/v1");
    else setBaseUrl("https://api.openai.com/v1");
  }
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true); setError(null);
    try { await api.createProvider({ name, kind, base_url: baseUrl, api_key: key }); await onChanged(); setName(""); setKey(""); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save provider."); }
    finally { setBusy(false); }
  }
  const editing = providers.find((provider) => provider.id === editingId) ?? null;
  return <div className="modal-backdrop" role="presentation"><section className="modal provider-modal" role="dialog" aria-modal="true" aria-labelledby="provider-title"><header><div><h2 id="provider-title">Providers</h2><p>Credentials are encrypted on the server and never returned to this device.</p></div><button className="icon-button" aria-label="Close" onClick={onClose}><X size={20} /></button></header><div className="provider-list">{providers.length === 0 ? <p className="muted">No providers configured.</p> : providers.map((provider) => <div className={`provider-card ${provider.id === editingId ? "selected" : ""}`} key={provider.id}><button type="button" className="provider-card-main" onClick={() => setEditingId(provider.id)}><span className="provider-avatar">{provider.name.slice(0, 1).toUpperCase()}</span><span className="provider-copy"><strong>{provider.name}</strong><span>{provider.base_url} · {provider.models.length} models</span></span></button><button className="icon-button danger" aria-label={`Delete ${provider.name}`} onClick={async () => { await api.deleteProvider(provider.id); if (editingId === provider.id) setEditingId(null); await onChanged(); }}><Trash2 size={17} /></button></div>)}</div>{editing && <ProviderSettings key={`${editing.id}-${revision}`} provider={editing} onChanged={onChanged} onError={setError} />}<form className="provider-form" onSubmit={submit}><h3>Add provider</h3><label>Provider API format<div className="kind-toggle" role="radiogroup" aria-label="Provider API format"><button type="button" role="radio" aria-checked={kind === "openai_compatible"} className={kind === "openai_compatible" ? "selected" : ""} onClick={() => setProviderKind("openai_compatible")}>OpenAI-compatible</button><button type="button" role="radio" aria-checked={kind === "openai_responses"} className={kind === "openai_responses" ? "selected" : ""} onClick={() => setProviderKind("openai_responses")}>OpenAI Responses</button><button type="button" role="radio" aria-checked={kind === "anthropic"} className={kind === "anthropic" ? "selected" : ""} onClick={() => setProviderKind("anthropic")}>Anthropic</button></div></label><label>Name<input required value={name} onChange={(event) => setName(event.target.value)} placeholder={kind === "anthropic" ? "Anthropic" : "OpenAI, DeepSeek, Qwen..."} /></label><label>Base URL<input type="url" required value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" required value={key} onChange={(event) => setKey(event.target.value)} /></label><p className="muted">Both API roots and versioned `/v1` base URLs are accepted. Add models below after saving.</p>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></form></section></div>;
}

type ModelDraft = { key: string; id?: string; group_name: string; model: string; kind: ProviderKind; context_window: number; supports_images: boolean; chain_context: boolean; hosted_tools: boolean };

const KIND_OPTIONS: [ProviderKind, string][] = [["openai_compatible", "OpenAI Chat"], ["openai_responses", "OpenAI Responses"], ["anthropic", "Anthropic"]];

function KindToggle({ value, label, onChange }: { value: ProviderKind; label: string; onChange: (value: ProviderKind) => void }) {
  return <div className="kind-toggle" role="radiogroup" aria-label={label}>{KIND_OPTIONS.map(([option, text]) => <button type="button" key={option} role="radio" aria-checked={value === option} className={value === option ? "selected" : ""} onClick={() => onChange(option)}>{text}</button>)}</div>;
}

/**
 * The whole provider page is one form: credentials and every model row are edited
 * locally and written together, so nothing can be left half-saved.
 */
function ProviderSettings({ provider, onChanged, onError }: { provider: Provider; onChanged: () => Promise<void>; onError: (value: string | null) => void }) {
  const [name, setName] = useState(provider.name);
  const [kind, setKind] = useState<ProviderKind>(provider.kind);
  const [baseUrl, setBaseUrl] = useState(provider.base_url);
  const [key, setKey] = useState("");
  const [drafts, setDrafts] = useState<ModelDraft[]>(provider.models.map((item) => ({ ...item, key: item.id })));
  const [removed, setRemoved] = useState<string[]>([]);
  const [adding, setAdding] = useState({ group_name: "General", model: "", kind: provider.kind, context_window: 128000, supports_images: false, chain_context: true, hosted_tools: true });
  const [busy, setBusy] = useState(false);
  const originals = new Map(provider.models.map((item) => [item.id, item]));
  const modelChanged = (item: ModelDraft) => {
    const before = item.id ? originals.get(item.id) : undefined;
    return !before || before.group_name !== item.group_name || before.model !== item.model || before.kind !== item.kind || before.context_window !== item.context_window || before.supports_images !== item.supports_images || before.chain_context !== item.chain_context || before.hosted_tools !== item.hosted_tools;
  };
  const dirty = name !== provider.name || kind !== provider.kind || baseUrl !== provider.base_url || key.trim() !== "" || removed.length > 0 || drafts.some(modelChanged);
  function update(key: string, patch: Partial<ModelDraft>) { setDrafts((current) => current.map((item) => item.key === key ? { ...item, ...patch } : item)); }
  function addModel() {
    if (!adding.model.trim()) return;
    setDrafts((current) => [...current, { ...adding, key: `new-${current.length}-${Date.now()}` }]);
    setAdding({ ...adding, model: "" });
  }
  function discard() {
    setDrafts(provider.models.map((item) => ({ ...item, key: item.id })));
    setRemoved([]);
    setName(provider.name);
    setKind(provider.kind);
    setBaseUrl(provider.base_url);
    setKey("");
    onError(null);
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!dirty || busy) return;
    setBusy(true);
    onError(null);
    try {
      if (name !== provider.name || kind !== provider.kind || baseUrl !== provider.base_url || key.trim()) await api.updateProvider(provider.id, { name: name.trim(), kind, base_url: baseUrl.trim(), api_key: key.trim() || undefined });
      for (const item of drafts) {
        const payload = { group_name: item.group_name.trim() || "General", model: item.model.trim(), kind: item.kind, context_window: item.context_window, supports_images: item.supports_images, chain_context: item.chain_context, hosted_tools: item.hosted_tools };
        if (!item.id) await api.createProviderModel(provider.id, payload);
        else if (modelChanged(item)) await api.updateProviderModel(provider.id, item.id, payload);
      }
      for (const id of removed) await api.deleteProviderModel(provider.id, id);
      setKey("");
      await onChanged();
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : "Could not save the provider.");
    } finally {
      setBusy(false);
    }
  }
  return <form className="provider-form" onSubmit={save}><h3>Edit provider</h3><label>Provider API format<KindToggle value={kind} label="Provider API format" onChange={setKind} /></label><label>Name<input value={name} onChange={(event) => setName(event.target.value)} /></label><label>Base URL<input type="url" value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" value={key} onChange={(event) => setKey(event.target.value)} placeholder="Leave blank to keep the current key" /></label><section className="provider-models"><h3>Configured models</h3>{drafts.map((item) => <div className="model-config-row" key={item.key}><input value={item.group_name} aria-label="Model group" onChange={(event) => update(item.key, { group_name: event.target.value })} /><input value={item.model} aria-label="Model name" onChange={(event) => update(item.key, { model: event.target.value })} /><KindToggle value={item.kind} label={`${item.model || "Model"} API format`} onChange={(value) => update(item.key, { kind: value })} /><input value={item.context_window} type="number" min="4096" max="2000000" aria-label="Maximum context tokens" onChange={(event) => update(item.key, { context_window: Number(event.target.value) })} /><label className="model-toggle" title="This model can analyze images"><input type="checkbox" checked={item.supports_images} onChange={(event) => update(item.key, { supports_images: event.target.checked })} />Vision</label>{item.kind === "openai_responses" && <label className="model-toggle" title="Reuse the conversation the provider already stores instead of resending it every turn"><input type="checkbox" checked={item.chain_context} onChange={(event) => update(item.key, { chain_context: event.target.checked })} />Reuse context</label>}{item.kind !== "openai_compatible" && <label className="model-toggle" title="Let the provider run its own search, page reading and code execution. Turn off to fall back to our SearXNG ReAct loop, or when the gateway rejects the tools."><input type="checkbox" checked={item.hosted_tools} onChange={(event) => update(item.key, { hosted_tools: event.target.checked })} />Hosted tools</label>}<button type="button" className="icon-button danger" aria-label={`Remove ${item.model}`} onClick={() => { setDrafts((current) => current.filter((row) => row.key !== item.key)); if (item.id) setRemoved((current) => [...current, item.id as string]); }}><Trash2 size={15} /></button></div>)}<div className="add-model"><input value={adding.group_name} aria-label="New model group" onChange={(event) => setAdding({ ...adding, group_name: event.target.value })} placeholder="Group" /><input value={adding.model} aria-label="New model name" onChange={(event) => setAdding({ ...adding, model: event.target.value })} placeholder="Model name" /><KindToggle value={adding.kind} label="New model API format" onChange={(value) => setAdding({ ...adding, kind: value })} /><input value={adding.context_window} type="number" min="4096" max="2000000" aria-label="New model context tokens" onChange={(event) => setAdding({ ...adding, context_window: Number(event.target.value) })} /><label className="model-toggle"><input type="checkbox" checked={adding.supports_images} onChange={(event) => setAdding({ ...adding, supports_images: event.target.checked })} />Vision</label>{adding.kind === "openai_responses" && <label className="model-toggle"><input type="checkbox" checked={adding.chain_context} onChange={(event) => setAdding({ ...adding, chain_context: event.target.checked })} />Reuse context</label>}{adding.kind !== "openai_compatible" && <label className="model-toggle" title="Let the provider run its own search, page reading and code execution. Turn off to fall back to our SearXNG ReAct loop, or when the gateway rejects the tools."><input type="checkbox" checked={adding.hosted_tools} onChange={(event) => setAdding({ ...adding, hosted_tools: event.target.checked })} />Hosted tools</label>}<button type="button" className="text-button small" onClick={addModel}>Add model</button></div></section><div className="provider-save">{dirty && <button type="button" className="text-button" onClick={discard}>Discard changes</button>}<button className="primary" disabled={busy || !dirty}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></div></form>;
}


function DictionaryPopover({ word, anchor, onClose }: { word: string; anchor: { x: number; y: number }; onClose: () => void }) {
  const dictionaries = ["russian_en", "german_en", "english_zh"] as const;
  const popoverRef = useRef<HTMLElement>(null);
  type Dictionary = typeof dictionaries[number];
  const [position, setPosition] = useState({ left: anchor.x, top: anchor.y, maxHeight: window.innerHeight - anchor.y - 12 });
  const [dictionary, setDictionary] = useState<Dictionary>(() => { const stored = localStorage.getItem("malim-dictionary"); return dictionaries.includes(stored as Dictionary) ? stored as Dictionary : (/[Ѐ-ӿ]/.test(word) ? "russian_en" : "english_zh"); }); const [data, setData] = useState<DictionaryResponse | null>(null); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => { localStorage.setItem("malim-dictionary", dictionary); }, [dictionary]);
  useEffect(() => {
    const place = () => {
      const rect = popoverRef.current?.getBoundingClientRect();
      if (!rect) return;
      const margin = 12;
      const left = Math.max(margin, Math.min(anchor.x - 12, window.innerWidth - rect.width - margin));
      setPosition({ left, top: anchor.y + 8, maxHeight: Math.max(56, window.innerHeight - anchor.y - 20) });
    };
    const frame = window.requestAnimationFrame(place); window.addEventListener("resize", place);
    return () => { window.cancelAnimationFrame(frame); window.removeEventListener("resize", place); };
  }, [anchor.x, anchor.y, word, data, busy]);
  useEffect(() => {
    const outside = (event: PointerEvent) => { if (!popoverRef.current?.contains(event.target as Node)) onClose(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("pointerdown", outside, true); document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("keydown", escape); };
  }, [onClose]);
  useEffect(() => { let live = true; setBusy(true); setError(null); setData(null); void api.dictionary(word, dictionary).then((response) => { if (live) setData(response); }).catch((cause) => { if (live) setError(cause instanceof Error ? cause.message : "Dictionary lookup failed."); }).finally(() => { if (live) setBusy(false); }); return () => { live = false; }; }, [word, dictionary]);
  return <aside ref={popoverRef} className="dictionary-popover" role="dialog" aria-label={`Dictionary lookup for ${word}`} style={position}><header><div><strong>{word}</strong><select value={dictionary} onChange={(event) => setDictionary(event.target.value as Dictionary)}><option value="english_zh">English - Chinese</option><option value="german_en">German - English</option><option value="russian_en">Russian - English</option></select></div><button className="icon-button" aria-label="Close dictionary" onClick={onClose}><X size={17} /></button></header>{busy && <p className="dictionary-loading"><LoaderCircle className="spin" size={16} />Looking up...</p>}{error && <p className="form-error">{error}</p>}{data?.entries.map((entry, index) => <DictionaryEntryView key={`${entry.headword}-${index}`} entry={entry} dictionary={dictionary} />)}{data && data.entries.length === 0 && <p className="muted">No local entry found.</p>}</aside>;
}

function DictionaryEntryView({ entry, dictionary }: { entry: DictionaryResponse["entries"][number]; dictionary: DictionaryResponse["dictionary"] }) {
  if (entry.definition_html) return <article className="dictionary-entry malim-entry" dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(entry.definition_html, { ADD_ATTR: ["target"] }) }} />;
  return <article className="dictionary-entry structured"><div className="dictionary-head"><div><h3>{entry.headword}</h3>{entry.pronunciation && <span>/{entry.pronunciation}/</span>}</div>{entry.labels.length > 0 && <small>{entry.labels.join(" · ")}</small>}</div>{entry.forms.length > 0 && <div className="dictionary-forms"><b>Forms</b><div className="dictionary-pills">{entry.forms.slice(0, 12).map((value) => <span key={value}>{value}</span>)}</div></div>}{entry.translations.length > 0 && <section className="dictionary-senses"><h4>Translations</h4><ol>{entry.translations.map((value) => <li key={value}>{value}</li>)}</ol></section>}{entry.definitions.length > 0 && <section className="dictionary-senses"><h4>{dictionary === "german_en" ? "German entry" : "Definition"}</h4><ol>{entry.definitions.slice(0, 12).map((value, sense) => <li key={`${sense}-${value}`}>{value}</li>)}</ol></section>}</article>;
}
