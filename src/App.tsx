import { Component, FormEvent, ReactNode, useEffect, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { Bot, Check, ChevronsUpDown, CircleAlert, Copy, Edit3, LoaderCircle, LogOut, Menu, MessageSquare, PanelLeftClose, PanelLeftOpen, Search, Settings2, SquarePen, Trash2, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark, oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";
import "./additions.css";
import { api, setSession, subscribeSession } from "./api";
import { useAppStore } from "./store";
import type { Conversation, DictionaryResponse, GenerationSettings, Message, Provider, ProviderKind, ProviderModel, Session } from "./types";

const now = () => new Date().toISOString();
const defaultGeneration: GenerationSettings = { temperature: 0.7, reasoning_effort: "medium", enable_markdown: true, stream: true };
function uuid() {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  return [...bytes].map((value, index) => `${[4, 6, 8, 10].includes(index) ? "-" : ""}${value.toString(16).padStart(2, "0")}`).join("");
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
  const state = useAppStore();
  const { conversations, providers, activeId, messages, sidebarOpen, busy, error } = state;
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
  const activeConversation = activeId ? conversations.find((item) => item.id === activeId) ?? null : null;

  useEffect(() => { void bootstrap(); }, []);
  useEffect(() => { if (activeId && !messages[activeId]) void loadMessages(activeId); }, [activeId]);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => setTheme(media.matches ? "dark" : "light");
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, []);
  useEffect(() => { if (activeConversation) setGeneration(activeConversation.generation_settings ?? defaultGeneration); }, [activeConversation?.id]);
  useEffect(() => () => { Object.values(generationTimers.current).forEach((timer) => window.clearTimeout(timer)); }, []);

  async function bootstrap() {
    try {
      const [chats, configured] = await Promise.all([api.conversations(), api.providers()]);
      state.setConversations(chats.items);
      state.setProviders(configured);
      if (!useAppStore.getState().activeId && chats.items[0]) state.setActiveId(chats.items[0].id);
    } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Unable to load your workspace."); }
  }
  async function loadMessages(id: string) {
    try { state.setMessages(id, (await api.messages(id)).items); }
    catch (cause) { state.setError(cause instanceof Error ? cause.message : "Unable to load messages."); }
  }
  async function newChat() {
    if (!providers.length) { setProviderOpen(true); state.setError("Add a provider before creating a chat."); return; }
    try {
      const conversation = await api.createConversation({ provider_id: providers[0].id, model: providers[0].models[0]?.model ?? providers[0].default_model });
      state.setConversations([conversation, ...useAppStore.getState().conversations]);
      state.setMessages(conversation.id, []);
      state.setActiveId(conversation.id);
      setSidebarCollapsed(false);
    } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Unable to create a conversation."); }
  }
  async function archive(id: string) {
    const before = useAppStore.getState().conversations;
    state.setConversations(before.filter((item) => item.id !== id));
    if (activeId === id) state.setActiveId(before.find((item) => item.id !== id)?.id ?? null);
    try { await api.updateConversation(id, { archived: true }); }
    catch (cause) { state.setConversations(before); state.setError(cause instanceof Error ? cause.message : "Unable to archive conversation."); }
  }
  async function changeModel(providerId: string, model: string) {
    if (!activeConversation || !model.trim()) return;
    setModelChanging(true);
    try {
      const updated = await api.updateConversation(activeConversation.id, { provider_id: providerId, model: model.trim() });
      state.setConversations(useAppStore.getState().conversations.map((item) => item.id === updated.id ? updated : item));
      setModelMenuOpen(false);
    } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Unable to change the model."); }
    finally { setModelChanging(false); }
  }
  function signOut() { setSession(null); }
  function changeGeneration(next: GenerationSettings) {
    if (!activeConversation) return;
    const conversationId = activeConversation.id;
    setGeneration(next);
    state.setConversations(useAppStore.getState().conversations.map((conversation) => conversation.id === conversationId ? { ...conversation, generation_settings: next } : conversation));
    window.clearTimeout(generationTimers.current[conversationId]);
    generationTimers.current[conversationId] = window.setTimeout(() => { void api.updateConversation(conversationId, { generation_settings: next }).catch((cause) => state.setError(cause instanceof Error ? cause.message : "Unable to save conversation parameters.")); }, 350);
  }
  async function sendMessage(conversationId: string, content: string, search: boolean, options = generation) {
    const mutationId = uuid();
    const optimistic: Message = { id: `local-${mutationId}`, conversation_id: conversationId, sequence: Number.MAX_SAFE_INTEGER - Date.now(), client_mutation_id: mutationId, role: "user", content, reasoning_content: "", content_format: "markdown", status: "pending", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now(), optimistic: true };
    let sent: Message | null = null;
    let pendingId: string | null = null;
    state.upsertMessage(conversationId, optimistic);
    state.setBusy(true);
    try {
      sent = await api.createMessage(conversationId, content, mutationId, search);
      state.upsertMessage(conversationId, sent);
      pendingId = `assistant-${mutationId}`;
      state.upsertMessage(conversationId, { ...sent, id: pendingId, sequence: sent.sequence + 0.5, client_mutation_id: null, role: "assistant", content: "", reasoning_content: "", status: "streaming", model: null, token_count: 0, search_sources: [] });
      const answer = options.stream ? await api.respondStream(conversationId, sent.id, search, options, (event) => { const current = useAppStore.getState().messages[conversationId]?.find((item) => item.id === pendingId); if (current) state.upsertMessage(conversationId, event.type === "reasoning" ? { ...current, reasoning_content: current.reasoning_content + (event.delta ?? "") } : { ...current, content: current.content + (event.delta ?? "") }); }) : await api.respond(conversationId, sent.id, search, options);
      state.removeMessage(conversationId, pendingId);
      state.upsertMessage(conversationId, answer);
      state.setConversations((await api.conversations()).items);
    } catch (cause) {
      if (pendingId) state.removeMessage(conversationId, pendingId);
      state.upsertMessage(conversationId, { ...(sent ?? optimistic), status: "error", optimistic: !sent, updated_at: now() });
      state.upsertMessage(conversationId, { id: `error-${mutationId}`, conversation_id: conversationId, sequence: (sent?.sequence ?? optimistic.sequence) + 0.1, client_mutation_id: null, role: "assistant", content: "The response could not be generated. Use retry to send the message again.", reasoning_content: "", content_format: "markdown", status: "error", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now(), optimistic: true, retry_message_id: (sent ?? optimistic).id });
      state.setError(cause instanceof Error ? cause.message : "Message delivery failed.");
    } finally { state.setBusy(false); }
  }
  async function retryMessage(message: Message) {
    if (!activeId) return;
    const target = message.retry_message_id ? (messages[activeId]?.find((item) => item.id === message.retry_message_id) ?? message) : message;
    if (message.retry_message_id) state.removeMessage(activeId, message.id);
    if (target.id.startsWith("local-")) { await sendMessage(activeId, target.content, searchEnabled); return; }
    const pendingId = `retry-${target.id}`;
    state.upsertMessage(activeId, { ...target, status: "pending", updated_at: now() });
    state.upsertMessage(activeId, { ...target, id: pendingId, sequence: target.sequence + 0.5, client_mutation_id: null, role: "assistant", content: "", reasoning_content: "", status: "streaming", model: null, token_count: 0, search_sources: [] });
    state.setBusy(true);
    try {
      const answer = generation.stream ? await api.respondStream(activeId, target.id, searchEnabled, generation, (event) => { const current = useAppStore.getState().messages[activeId]?.find((item) => item.id === pendingId); if (current) state.upsertMessage(activeId, event.type === "reasoning" ? { ...current, reasoning_content: current.reasoning_content + (event.delta ?? "") } : { ...current, content: current.content + (event.delta ?? "") }); }) : await api.respond(activeId, target.id, searchEnabled, generation);
      state.removeMessage(activeId, pendingId);
      state.upsertMessage(activeId, { ...target, status: "complete", updated_at: now() });
      state.upsertMessage(activeId, answer);
    } catch (cause) {
      state.removeMessage(activeId, pendingId);
      state.upsertMessage(activeId, { ...target, status: "error", updated_at: now() });
      state.setError(cause instanceof Error ? cause.message : "Retry failed.");
    } finally { state.setBusy(false); }
  }
  async function editMessage(message: Message, content: string) {
    if (!activeId) return;
    const before = messages[activeId] ?? [];
    state.upsertMessage(activeId, { ...message, content });
    try { state.upsertMessage(activeId, await api.updateMessage(activeId, message.id, content)); }
    catch (cause) { state.setMessages(activeId, before); state.setError(cause instanceof Error ? cause.message : "Could not edit message."); }
  }
  async function deleteMessage(message: Message) {
    if (!activeId) return;
    const before = messages[activeId] ?? [];
    state.removeMessage(activeId, message.id);
    if (message.optimistic || message.id.startsWith("local-") || message.id.startsWith("error-") || message.id.startsWith("assistant-") || message.id.startsWith("retry-")) return;
    try { await api.deleteMessage(activeId, message.id); }
    catch (cause) { state.setMessages(activeId, before); state.setError(cause instanceof Error ? cause.message : "Could not delete message."); }
  }

  async function compactConversation() { if (!activeId || compacting) return; setCompacting(true); try { const result = await api.compact(activeId); state.upsertMessage(activeId, result.message); state.setConversations((await api.conversations()).items); } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Compaction failed."); } finally { setCompacting(false); } }
  return <main className={`app-shell ${theme} ${sidebarCollapsed ? "sidebar-is-collapsed" : ""}`}><Sidebar conversations={conversations} activeId={activeId} open={sidebarOpen} collapsed={sidebarCollapsed} onNew={() => void newChat()} onSelect={state.setActiveId} onRename={async (id, title) => { try { const updated = await api.updateConversation(id, { title }); state.setConversations(useAppStore.getState().conversations.map((item) => item.id === id ? updated : item)); } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Unable to rename conversation."); } }} onSettings={() => setProviderOpen(true)} onClose={() => state.setSidebarOpen(false)} onCollapse={() => setSidebarCollapsed(true)} userName={state.session?.user.display_name ?? "Account"} onSignOut={signOut} />{sidebarOpen && <div className="sidebar-backdrop" onClick={() => state.setSidebarOpen(false)} />}<section className="chat-shell"><header className="chat-header"><button className="icon-button desktop-reopen" aria-label="Open navigation" onClick={() => setSidebarCollapsed(false)}><PanelLeftOpen size={19} /></button><button className="icon-button mobile-only" aria-label="Open navigation" onClick={() => state.setSidebarOpen(true)}><Menu size={20} /></button><div className="header-title"><span>{activeConversation?.title ?? "malim_chat"}</span></div>{activeConversation && <ModelSelector conversation={activeConversation} providers={providers} open={modelMenuOpen} busy={modelChanging} onToggle={() => setModelMenuOpen(!modelMenuOpen)} onChange={changeModel} />}</header>{error && <div className="notice"><CircleAlert size={16} /><span>{error}</span><button className="icon-button" aria-label="Dismiss" onClick={() => state.setError(null)}><X size={16} /></button></div>}<ConversationView conversation={activeConversation} messages={activeId ? messages[activeId] ?? [] : []} providers={providers} searchEnabled={searchEnabled} busy={busy} compacting={compacting} theme={theme} onToggleSearch={() => setSearchEnabled(!searchEnabled)} generation={generation} onGenerationChange={changeGeneration} onSend={(content) => activeId ? sendMessage(activeId, content, searchEnabled) : Promise.resolve()} onEdit={editMessage} onDelete={deleteMessage} onRetry={retryMessage} onLookup={(word, anchor) => setLookup({ word, ...anchor })} onCompact={compactConversation} /><RightNavigator messages={activeId ? messages[activeId] ?? [] : []} /></section>{providerOpen && <ProviderDialog providers={providers} onClose={() => setProviderOpen(false)} onChanged={async () => { state.setProviders(await api.providers()); }} />}{lookup && <DictionaryPopover word={lookup.word} anchor={lookup} onClose={() => setLookup(null)} />}</main>;
}

function Sidebar({ conversations, activeId, open, collapsed, onNew, onSelect, onRename, onSettings, onClose, onCollapse, userName, onSignOut }: { conversations: Conversation[]; activeId: string | null; open: boolean; collapsed: boolean; onNew: () => void; onSelect: (id: string) => void; onRename: (id: string, title: string) => Promise<void>; onSettings: () => void; onClose: () => void; onCollapse: () => void; userName: string; onSignOut: () => void }) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  function startRename(conversation: Conversation) { setRenamingId(conversation.id); setDraft(conversation.title); }
  async function finishRename(id: string) { const title = draft.trim(); if (title) await onRename(id, title); setRenamingId(null); }
  return <aside className={`sidebar ${open ? "open" : ""} ${collapsed ? "collapsed" : ""}`}><div className="sidebar-top"><div className="wordmark"><Bot size={20} /><span>malim_chat</span></div><button className="icon-button desktop-only" aria-label="Collapse navigation" onClick={onCollapse}><PanelLeftClose size={19} /></button><button className="icon-button mobile-only" aria-label="Close navigation" onClick={onClose}><X size={19} /></button></div><button className="new-chat" type="button" onClick={onNew}><SquarePen size={17} />New chat</button><nav aria-label="Conversation history" className="history"><p>Conversations</p>{conversations.map((conversation) => <div className={`conversation-row ${conversation.id === activeId ? "selected" : ""}`} key={conversation.id}>{renamingId === conversation.id ? <form className="conversation-rename" onSubmit={(event) => { event.preventDefault(); void finishRename(conversation.id); }}><input autoFocus value={draft} maxLength={160} onChange={(event) => setDraft(event.target.value)} onBlur={() => void finishRename(conversation.id)} /></form> : <button type="button" onClick={() => onSelect(conversation.id)}><MessageSquare size={16} /><span>{conversation.title}</span></button>}<button type="button" className="row-action" aria-label={`Rename ${conversation.title}`} title="Rename conversation" onMouseDown={(event) => event.preventDefault()} onClick={() => startRename(conversation)}><Edit3 size={15} /></button></div>)}</nav><div className="sidebar-bottom"><button className="settings-button" type="button" onClick={onSettings}><Settings2 size={17} />Providers & settings</button><div className="account-row"><div className="avatar">{userName.slice(0, 1).toUpperCase()}</div><span>{userName}</span><button type="button" className="row-action" aria-label="Sign out" title="Sign out" onClick={onSignOut}><LogOut size={17} /></button></div></div></aside>;
}

function ModelSelector({ conversation, providers, open, busy, onToggle, onChange }: { conversation: Conversation; providers: Provider[]; open: boolean; busy: boolean; onToggle: () => void; onChange: (providerId: string, model: string) => Promise<void> }) {
  const activeProvider = providers.find((provider) => provider.id === conversation.model_provider_id);
  const [providerId, setProviderId] = useState(conversation.model_provider_id ?? providers[0]?.id ?? "");
  const [model, setModel] = useState(conversation.model ?? activeProvider?.models[0]?.model ?? "");
  useEffect(() => { setProviderId(conversation.model_provider_id ?? providers[0]?.id ?? ""); setModel(conversation.model ?? activeProvider?.models[0]?.model ?? ""); }, [conversation.id, conversation.model_provider_id, conversation.model, activeProvider?.models, providers]);
  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const groups = (selectedProvider?.models ?? []).reduce<Record<string, ProviderModel[]>>((all, item) => { (all[item.group_name] ??= []).push(item); return all; }, {});
  return <div className="model-selector"><button type="button" className="model-button" aria-expanded={open} onClick={onToggle}><span>{model || "Choose model"}</span><ChevronsUpDown size={15} /></button>{open && <div className="model-popover"><div className="model-picker-heading">Provider</div><div className="provider-options">{providers.map((provider) => <button type="button" key={provider.id} className={provider.id === providerId ? "selected" : ""} onClick={() => { setProviderId(provider.id); setModel(provider.models[0]?.model ?? ""); }}>{provider.name}</button>)}</div><div className="model-picker-heading">Model</div><div className="configured-models">{Object.entries(groups).map(([group, items]) => <section key={group}><h4>{group}</h4>{items.map((item) => <button type="button" key={item.id} className={item.model === model ? "selected" : ""} onClick={() => { setModel(item.model); void onChange(providerId, item.model); }}>{item.model}</button>)}</section>)}</div></div>}</div>;
}

function ConversationView({ conversation, messages, providers, searchEnabled, busy, compacting, theme, generation, onGenerationChange, onToggleSearch, onSend, onEdit, onDelete, onRetry, onLookup, onCompact }: { conversation: Conversation | null; messages: Message[]; providers: Provider[]; searchEnabled: boolean; busy: boolean; compacting: boolean; theme: "light" | "dark"; generation: GenerationSettings; onGenerationChange: (value: GenerationSettings) => void; onToggleSearch: () => void; onSend: (content: string) => Promise<void>; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void; onCompact: () => Promise<void> }) {
  const [input, setInput] = useState("");
  const endRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { endRef.current?.scrollIntoView({ behavior: "smooth" }); }, [messages.length, busy]);
  useEffect(() => { const node = inputRef.current; if (!node) return; node.style.height = "auto"; node.style.height = `${Math.min(node.scrollHeight, 180)}px`; }, [input]);
  if (!conversation) return <section className="empty-state"><div className="empty-logo"><Bot size={34} /></div><h1>How can I help you today?</h1><p>Start a new conversation to work with your configured AI providers.</p></section>;
  const usage = Math.min(100, Math.round((conversation.context_tokens / conversation.context_window) * 100));
  const hasActiveProvider = providers.some((provider) => provider.id === conversation.model_provider_id && provider.models.some((item) => item.model === conversation.model));
  function send() { const text = input.trim(); if (!text || busy || !hasActiveProvider) return; setInput(""); void onSend(text); }
  function submit(event: FormEvent) { event.preventDefault(); send(); }
  return <><section className="message-list">{messages.map((message, index) => <MessageBubble key={message.id} message={message} markdownEnabled={generation.enable_markdown} theme={theme} isLast={index === messages.length - 1} onEdit={onEdit} onDelete={onDelete} onRetry={onRetry} onLookup={onLookup} />)}<div ref={endRef} /></section><footer className="composer-wrap"><div className="context-meter"><span>Context {conversation.context_tokens.toLocaleString()} / {conversation.context_window.toLocaleString()} tokens</span><div><i style={{ width: `${usage}%` }} /></div><button type="button" disabled={compacting} onClick={() => void onCompact()} title="Compact previous context">{compacting ? <><LoaderCircle className="spin" size={12} />Compacting</> : "Compact"}</button></div><form className="composer" onSubmit={submit}><textarea ref={inputRef} value={input} onChange={(event) => setInput(event.target.value)} placeholder={hasActiveProvider ? "Message malim_chat" : "Choose a model from the conversation header"} disabled={!hasActiveProvider || busy || compacting} rows={1} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); send(); } }} /><div className="composer-controls"><button type="button" className={searchEnabled ? "search-chip enabled" : "search-chip"} aria-pressed={searchEnabled} onClick={onToggleSearch}><Search size={14} />{searchEnabled ? "Web search on" : "Web search off"}</button><GenerationButton generation={generation} onChange={onGenerationChange} kind={providers.find((item) => item.id === conversation.model_provider_id)?.models.find((item) => item.model === conversation.model)?.kind} model={conversation.model ?? ""} /><button type="submit" className="send-button" aria-label="Send message" disabled={!input.trim() || busy || compacting || !hasActiveProvider}>{busy ? <LoaderCircle className="spin" size={18} /> : <Check size={18} />}</button></div></form><p className="disclaimer">AI responses may be inaccurate. Review important information.</p></footer></>;
}

function RightNavigator({ messages }: { messages: Message[] }) {
  const questions = messages.filter((message) => message.role === "user");
  return <aside className="right-navigator" aria-label="Conversation navigation"><div className="right-nav-inner"><strong>In this chat</strong>{questions.length === 0 ? <span className="muted">No questions yet</span> : questions.map((message) => <button key={message.id} onClick={() => document.getElementById(`message-${message.id}`)?.scrollIntoView({ behavior: "smooth", block: "center" })}><span className="right-nav-label">{message.content}</span></button>)}</div></aside>;
}

function GenerationButton({ generation, onChange, kind, model }: { generation: GenerationSettings; onChange: (value: GenerationSettings) => void; kind?: ProviderKind; model: string }) {
  const [open, setOpen] = useState(false);
  const controlRef = useRef<HTMLDivElement>(null);
  const supportsReasoning = kind === "openai_compatible" && /(?:gpt-5|\bo[1-9]\b|codex|reasoner|thinking|deepseek-r1)/i.test(model);
  const effort = ["low", "medium", "high"] as const;
  useEffect(() => { const outside = (event: PointerEvent) => { if (!controlRef.current?.contains(event.target as Node)) setOpen(false); }; document.addEventListener("pointerdown", outside, true); return () => document.removeEventListener("pointerdown", outside, true); }, []);
  return <div ref={controlRef} className="generation-control"><button type="button" className="search-chip" onClick={() => setOpen(!open)} title="Generation parameters"><Settings2 size={14} />Parameters</button>{open && <div className="generation-popover"><label><span>Temperature <b>{generation.temperature.toFixed(1)}</b></span><input type="range" min="0" max="2" step="0.1" value={generation.temperature} onChange={(event) => onChange({ ...generation, temperature: Number(event.target.value) })} /></label>{supportsReasoning && <label><span>Reasoning effort <b>{generation.reasoning_effort}</b></span><input type="range" min="0" max="2" step="1" value={effort.indexOf(generation.reasoning_effort as typeof effort[number])} onChange={(event) => onChange({ ...generation, reasoning_effort: effort[Number(event.target.value)] })} /><small>Low <i /> Medium <i /> High</small></label>}<label className="markdown-toggle"><span>Enable Markdown</span><input type="checkbox" checked={generation.enable_markdown} onChange={(event) => onChange({ ...generation, enable_markdown: event.target.checked })} /></label><label className="markdown-toggle"><span>Stream output</span><input type="checkbox" checked={generation.stream} onChange={(event) => onChange({ ...generation, stream: event.target.checked })} /></label></div>}</div>;
}

function ThinkingBlock({ reasoning }: { reasoning: string }) {
  const [open, setOpen] = useState(false);
  const [contentHeight, setContentHeight] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) { setContentHeight(0); return; }
    const measure = () => { const node = scrollRef.current; if (node) setContentHeight(Math.min(node.scrollHeight, 300)); };
    measure();
    const observer = new ResizeObserver(measure);
    if (scrollRef.current) observer.observe(scrollRef.current);
    return () => observer.disconnect();
  }, [open, reasoning]);
  return <div className="thinking-block"><button type="button" className="thinking-summary" aria-expanded={open} onClick={() => setOpen(!open)}>Thinking</button><div className="thinking-collapse" style={{ maxHeight: open ? contentHeight : 0 }}><div ref={scrollRef} className="thinking-scroll"><div className="plain-content">{reasoning}</div></div></div></div>;
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
  return converted.replace(/\u0000(\d+)\u0000/g, (_, index) => blocks[Number(index)]);
}

function MessageBubble({ message, markdownEnabled, theme, isLast, onEdit, onDelete, onRetry, onLookup }: { message: Message; markdownEnabled: boolean; theme: "light" | "dark"; isLast: boolean; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [copied, setCopied] = useState(false);
  async function copyContent() {
    const text = content;
    try {
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
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch { /* clipboard unavailable */ }
  }
  function captureSelection() { const selected = window.getSelection()?.toString().trim() ?? ""; if (selected && selected.length <= 120) { const range = window.getSelection()?.rangeCount ? window.getSelection()?.getRangeAt(0).getBoundingClientRect() : undefined; onLookup(selected, { x: Math.min(window.innerWidth - 18, Math.max(18, range?.left ?? window.innerWidth / 2)), y: Math.min(window.innerHeight - 18, Math.max(18, range?.bottom ?? window.innerHeight / 2)) }); } }
  const legacy = splitLegacyThinking(message.content);
  const content = message.reasoning_content ? message.content : legacy.content;
  const reasoning = message.reasoning_content || legacy.reasoning;
  const renderMarkdown = markdownEnabled && message.role === "assistant" && message.content_format === "markdown";
  return <article id={`message-${message.id}`} className={`message ${message.role === "user" ? "user" : message.role === "summary" ? "summary" : "assistant"} ${message.status === "error" ? "message-error" : ""}`}><div className="message-avatar">{message.role === "user" ? "You" : <Bot size={17} />}</div><div className="message-body" onMouseUp={captureSelection} onTouchEnd={captureSelection}>{editing ? <div className="edit-box"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} /><button className="primary small" type="button" onClick={() => { void onEdit(message, draft); setEditing(false); }}>Save</button><button className="text-button small" type="button" onClick={() => { setDraft(message.content); setEditing(false); }}>Cancel</button></div> : <>{reasoning && <ThinkingBlock reasoning={reasoning} />}{message.status === "streaming" && !content && !reasoning ? <span className="typing"><i /><i /><i /></span> : content && <div className={`message-content ${renderMarkdown ? "markdown-content" : "plain-content"}`}>{renderMarkdown ? <ReactMarkdown remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex]} components={{ code({ className, children, ...props }) { const language = /language-(\w+)/.exec(className ?? "")?.[1]; const source = String(children).replace(/\n$/, ""); return language ? <SyntaxHighlighter language={language} style={theme === "dark" ? oneDark : oneLight} showLineNumbers wrapLongLines customStyle={{ margin: "0", background: "transparent", padding: "12px 0" }}>{source}</SyntaxHighlighter> : <code className={className} {...props}>{children}</code>; } }}>{preprocessLatex(content)}</ReactMarkdown> : content}</div>}</>}{isLast && message.status === "error" && (message.role === "user" || message.retry_message_id) && <button type="button" className="retry-button" onClick={() => void onRetry(message)}>Retry response</button>}{message.search_sources?.length > 0 && <div className="sources">{message.search_sources.slice(0, 3).map((source) => <a href={source.url} target="_blank" rel="noreferrer" key={source.url}>{source.title}</a>)}</div>}<div className="message-tools"><button type="button" className={copied ? "copied" : ""} title={copied ? "Copied" : "Copy"} onClick={() => void copyContent()}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>{!message.optimistic && message.role !== "system" && <button type="button" title="Edit" onClick={() => setEditing(true)}><Edit3 size={14} /></button>}<button type="button" title="Delete" onClick={() => void onDelete(message)}><Trash2 size={14} /></button></div></div></article>;
}

function splitLegacyThinking(value: string) {
  const match = /<think>([\s\S]*?)(?:<\/think>|$)/i.exec(value);
  return match ? { content: value.replace(match[0], "").trimStart(), reasoning: match[1].trim() } : { content: value, reasoning: "" };
}

function ProviderDialog({ providers, onClose, onChanged }: { providers: Provider[]; onClose: () => void; onChanged: () => Promise<void> }) {
  const [name, setName] = useState(""); const [baseUrl, setBaseUrl] = useState("https://api.openai.com"); const [key, setKey] = useState(""); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false); const [editingId, setEditingId] = useState<string | null>(providers[0]?.id ?? null);
  async function submit(event: FormEvent) { event.preventDefault(); setBusy(true); setError(null); try { await api.createProvider({ name, base_url: baseUrl, api_key: key }); await onChanged(); setName(""); setKey(""); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save provider."); } finally { setBusy(false); } }
  const editing = providers.find((provider) => provider.id === editingId) ?? null;
  return <div className="modal-backdrop" role="presentation"><section className="modal provider-modal" role="dialog" aria-modal="true" aria-labelledby="provider-title"><header><div><h2 id="provider-title">Providers</h2><p>Credentials are encrypted on the server and never returned to this device.</p></div><button className="icon-button" aria-label="Close" onClick={onClose}><X size={20} /></button></header><div className="provider-list">{providers.length === 0 ? <p className="muted">No providers configured.</p> : providers.map((provider) => <div className={`provider-card ${provider.id === editingId ? "selected" : ""}`} key={provider.id}><button type="button" className="provider-card-main" onClick={() => setEditingId(provider.id)}><span className="provider-avatar">{provider.name.slice(0, 1).toUpperCase()}</span><span className="provider-copy"><strong>{provider.name}</strong><span>{provider.models.length} configured models</span></span></button><button className="icon-button danger" aria-label={`Delete ${provider.name}`} onClick={async () => { await api.deleteProvider(provider.id); if (editingId === provider.id) setEditingId(null); await onChanged(); }}><Trash2 size={17} /></button></div>)}</div>{editing && <ProviderModelsEditor provider={editing} onChanged={onChanged} onError={setError} />}<form className="provider-form" onSubmit={submit}><h3>Add provider</h3><label>Name<input required value={name} onChange={(event) => setName(event.target.value)} placeholder="OpenAI, DeepSeek, Qwen..." /></label><label>Base URL<input type="url" required value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" required value={key} onChange={(event) => setKey(event.target.value)} /></label><p className="muted">Add your models below after saving; each model has its own API format.</p>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></form></section></div>;
}

function ProviderModelsEditor({ provider, onChanged, onError }: { provider: Provider; onChanged: () => Promise<void>; onError: (value: string | null) => void }) {
  const [group, setGroup] = useState("General"); const [model, setModel] = useState(""); const [kind, setKind] = useState<ProviderKind>("openai_compatible"); const [contextWindow, setContextWindow] = useState(128000); const [busy, setBusy] = useState(false);
  async function add(event: FormEvent) { event.preventDefault(); setBusy(true); onError(null); try { await api.createProviderModel(provider.id, { group_name: group, model, kind, context_window: contextWindow }); setModel(""); await onChanged(); } catch (cause) { onError(cause instanceof Error ? cause.message : "Could not add model."); } finally { setBusy(false); } }
  const groups = provider.models.reduce<Record<string, ProviderModel[]>>((all, item) => { (all[item.group_name] ??= []).push(item); return all; }, {});
  return <section className="provider-models"><h3>Configured models</h3>{Object.entries(groups).map(([label, items]) => <div className="model-group" key={label}><div className="model-group-head"><h4>{label}</h4><span>{items.length}</span></div>{items.map((item) => <ModelConfigRow key={item.id} providerId={provider.id} item={item} onChanged={onChanged} onError={onError} />)}</div>)}<form className="add-model" onSubmit={add}><input value={group} onChange={(event) => setGroup(event.target.value)} placeholder="Group" required /><input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Model name" required /><div className="kind-toggle" role="radiogroup" aria-label="API format"><button type="button" role="radio" aria-checked={kind === "openai_compatible"} className={kind === "openai_compatible" ? "selected" : ""} onClick={() => setKind("openai_compatible")}>OpenAI</button><button type="button" role="radio" aria-checked={kind === "anthropic"} className={kind === "anthropic" ? "selected" : ""} onClick={() => setKind("anthropic")}>Anthropic</button></div><input value={contextWindow} type="number" min="4096" max="2000000" aria-label="Maximum context tokens" onChange={(event) => setContextWindow(Number(event.target.value))} /><button className="primary small" disabled={busy}>Add model</button></form></section>;
}

function ModelConfigRow({ providerId, item, onChanged, onError }: { providerId: string; item: ProviderModel; onChanged: () => Promise<void>; onError: (value: string | null) => void }) {
  const [group, setGroup] = useState(item.group_name); const [model, setModel] = useState(item.model); const [kind, setKind] = useState<ProviderKind>(item.kind); const [contextWindow, setContextWindow] = useState(item.context_window);
  return <div className="model-config-row"><input value={group} aria-label="Model group" onChange={(event) => setGroup(event.target.value)} /><input value={model} aria-label="Model name" onChange={(event) => setModel(event.target.value)} /><div className="kind-toggle" role="radiogroup" aria-label="API format"><button type="button" role="radio" aria-checked={kind === "openai_compatible"} className={kind === "openai_compatible" ? "selected" : ""} onClick={() => setKind("openai_compatible")}>OpenAI</button><button type="button" role="radio" aria-checked={kind === "anthropic"} className={kind === "anthropic" ? "selected" : ""} onClick={() => setKind("anthropic")}>Anthropic</button></div><input value={contextWindow} type="number" min="4096" max="2000000" aria-label="Maximum context tokens" onChange={(event) => setContextWindow(Number(event.target.value))} /><button type="button" className="text-button small" onClick={async () => { try { await api.updateProviderModel(providerId, item.id, { group_name: group, model, kind, context_window: contextWindow }); await onChanged(); } catch (cause) { onError(cause instanceof Error ? cause.message : "Could not update model."); } }}>Save</button><button type="button" className="icon-button danger" aria-label={`Delete ${item.model}`} onClick={async () => { try { await api.deleteProviderModel(providerId, item.id); await onChanged(); } catch (cause) { onError(cause instanceof Error ? cause.message : "Could not delete model."); } }}><Trash2 size={15} /></button></div>;
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
