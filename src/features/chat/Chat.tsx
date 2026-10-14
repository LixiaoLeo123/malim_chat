import { useCallback, useEffect, useRef, useState } from "react";
import { CircleAlert, Menu, PanelLeftOpen, X } from "lucide-react";
import "../../additions.css";
import { api, setSession, subscribeSession, type StreamEvent } from "../../api";
import { useAppStore } from "../../store";
import type { GenerationSettings, Message, Session, ToolActivity } from "../../types";
import { androidApp, defaultGeneration, now, suppressLookup, updateToolEvents, uuid } from "../../lib/utils";
import { useEvent } from "../../hooks/use-event";
import { clearLookupHighlight } from "../dictionary/selection";
import { ChatErrorBoundary } from "../../components/common/error-boundary";
import { Auth } from "../auth/Auth";
import { Sidebar } from "./Sidebar";
import { ModelSelector } from "../providers/ModelSelector";
import { ConversationView } from "./ConversationView";
import { RightNavigator } from "./RightNavigator";
import { ProviderDialog } from "../providers/ProviderDialog";
import { DictionaryPopover } from "../dictionary/DictionaryPopover";

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

function Chat() {
  const {
    session, conversations, providers, activeId, messages, sidebarOpen, busy, error,
    appendConversations, prependMessages, setProviders, setConversations, setActiveId,
    setSidebarOpen, setBusy, setError, setMessages, upsertMessage, removeMessage,
  } = useAppStore();
  const [providerOpen, setProviderOpen] = useState(false);
  const [searchEnabled, setSearchEnabled] = useState(() => localStorage.getItem("malim-search-enabled") === "1");
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
  useEffect(() => { localStorage.setItem("malim-search-enabled", searchEnabled ? "1" : "0"); }, [searchEnabled]);
  useEffect(() => {
    const conversation = useAppStore.getState().conversations.find((item) => item.id === activeId);
    if (conversation) {
      const loaded = conversation.generation_settings ?? defaultGeneration;
      // A stored 0 is below the slider's floor of one round; clamp it so the value never strands
      // the thumb at 1 while the label reads 0.
      setGeneration(loaded.context_rounds === 0 ? { ...loaded, context_rounds: 1 } : loaded);
    }
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
    // Start where the user last was: the most recent conversation's model, not the
    // first provider's first model they probably never chose themselves.
    const previous = useAppStore.getState().conversations[0];
    const provider = providers.find((item) => item.id === previous?.model_provider_id) ?? providers[0];
    const model = previous?.model && previous.model_provider_id === provider.id
      ? previous.model
      : provider.models[0]?.model ?? provider.default_model;
    try {
      const conversation = await api.createConversation({ provider_id: provider.id, model });
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
  return <main className={`app-shell ${theme} ${sidebarCollapsed ? "sidebar-is-collapsed" : ""} ${androidApp ? "no-native-selection" : ""}`}><Sidebar conversations={conversations} activeId={activeId} open={sidebarOpen} collapsed={sidebarCollapsed} onNew={() => void newChat()} onSelect={setActiveId} onRename={async (id, title) => { try { const updated = await api.updateConversation(id, { title }); setConversations(useAppStore.getState().conversations.map((item) => item.id === id ? updated : item)); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to rename conversation."); } }} hasMore={conversationCursor !== null} loadingMore={loadingConversations} onLoadMore={loadMoreConversations} onToggleFavorite={toggleFavorite} onSettings={() => setProviderOpen(true)} onClose={() => setSidebarOpen(false)} onCollapse={() => setSidebarCollapsed(true)} userName={session?.user.display_name ?? "Account"} onSignOut={signOut} />{sidebarOpen && <div className="sidebar-backdrop" onClick={() => setSidebarOpen(false)} />}<section className="chat-shell"><header className="chat-header"><button className="icon-button desktop-reopen" aria-label="Open navigation" onClick={() => setSidebarCollapsed(false)}><PanelLeftOpen size={19} /></button><button className="icon-button mobile-only" aria-label="Open navigation" onClick={() => setSidebarOpen(true)}><Menu size={20} /></button><div className="header-title"><span>{activeConversation?.title ?? "malim_chat"}</span></div>{activeConversation && <ModelSelector conversation={activeConversation} providers={providers} open={modelMenuOpen} busy={modelChanging} onToggle={() => setModelMenuOpen(!modelMenuOpen)} onChange={changeModel} />}</header>{error && <div className="notice"><CircleAlert size={16} /><span>{error}</span><button className="icon-button" aria-label="Dismiss" onClick={() => setError(null)}><X size={16} /></button></div>}<ConversationView conversation={activeConversation} messages={activeId ? messages[activeId] ?? [] : []} hasOlder={Boolean(activeId && messageCursors[activeId])} loadingOlder={loadingOlder} onLoadOlder={loadOlderMessages} providers={providers} searchEnabled={searchEnabled} busy={busy} compacting={compacting} theme={theme} lookupActive={lookup !== null} streaming={streaming} onStop={stopStreaming} onToggleSearch={() => setSearchEnabled(!searchEnabled)} generation={generation} onGenerationChange={changeGeneration} onSend={(content, images) => activeId ? sendMessage(activeId, content, searchEnabled, generation, images ?? []) : Promise.resolve()} onEdit={handleEditMessage} onDelete={handleDeleteMessage} onRetry={handleRetryMessage} onLookup={handleLookup} onCompact={compactConversation} /><RightNavigator messages={activeId ? messages[activeId] ?? [] : []} /></section>{providerOpen && <ProviderDialog providers={providers} onClose={() => setProviderOpen(false)} onChanged={async () => { setProviders(await api.providers()); }} />}{lookup && <DictionaryPopover word={lookup.word} anchor={lookup} onClose={() => { clearLookupHighlight(); suppressLookup(lookup.word); setLookup(null); }} />}</main>;
}
