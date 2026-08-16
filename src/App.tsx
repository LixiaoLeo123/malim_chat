import { Component, FormEvent, ReactNode, useEffect, useRef, useState } from "react";
import { Archive, Bot, Check, ChevronsUpDown, CircleAlert, Copy, Edit3, Languages, LoaderCircle, LogOut, Menu, MessageSquare, PanelLeftClose, PanelLeftOpen, Search, Settings2, SquarePen, Trash2, X } from "lucide-react";
import { api, setSession, subscribeSession } from "./api";
import { useAppStore } from "./store";
import type { Conversation, DictionaryResponse, Message, Provider, Session } from "./types";

const now = () => new Date().toISOString();
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
  const [lookupWord, setLookupWord] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [modelChanging, setModelChanging] = useState(false);
  const activeConversation = activeId ? conversations.find((item) => item.id === activeId) ?? null : null;

  useEffect(() => { void bootstrap(); }, []);
  useEffect(() => { if (activeId && !messages[activeId]) void loadMessages(activeId); }, [activeId]);

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
      const conversation = await api.createConversation({ provider_id: providers[0].id, model: providers[0].default_model });
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
  async function sendMessage(conversationId: string, content: string, search: boolean) {
    const mutationId = uuid();
    const optimistic: Message = { id: `local-${mutationId}`, conversation_id: conversationId, sequence: Number.MAX_SAFE_INTEGER - Date.now(), client_mutation_id: mutationId, role: "user", content, content_format: "markdown", status: "pending", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now(), optimistic: true };
    let sent: Message | null = null;
    let pendingId: string | null = null;
    state.upsertMessage(conversationId, optimistic);
    state.setBusy(true);
    try {
      sent = await api.createMessage(conversationId, content, mutationId, search);
      state.upsertMessage(conversationId, sent);
      pendingId = `assistant-${mutationId}`;
      state.upsertMessage(conversationId, { ...sent, id: pendingId, sequence: sent.sequence + 0.5, client_mutation_id: null, role: "assistant", content: "", status: "streaming", model: null, token_count: 0, search_sources: [] });
      const answer = await api.respond(conversationId, sent.id, search);
      state.removeMessage(conversationId, pendingId);
      state.upsertMessage(conversationId, answer);
      state.setConversations((await api.conversations()).items);
    } catch (cause) {
      if (pendingId) state.removeMessage(conversationId, pendingId);
      state.upsertMessage(conversationId, { ...(sent ?? optimistic), status: "error", optimistic: !sent, updated_at: now() });
      state.upsertMessage(conversationId, { id: `error-${mutationId}`, conversation_id: conversationId, sequence: (sent?.sequence ?? optimistic.sequence) + 0.1, client_mutation_id: null, role: "assistant", content: "The response could not be generated. Use retry to send the message again.", content_format: "markdown", status: "error", model: null, token_count: 0, search_sources: [], edited_at: null, created_at: now(), updated_at: now() });
      state.setError(cause instanceof Error ? cause.message : "Message delivery failed.");
    } finally { state.setBusy(false); }
  }
  async function retryMessage(message: Message) {
    if (!activeId) return;
    if (message.id.startsWith("local-")) { await sendMessage(activeId, message.content, searchEnabled); return; }
    const pendingId = `retry-${message.id}`;
    state.upsertMessage(activeId, { ...message, status: "pending", updated_at: now() });
    state.upsertMessage(activeId, { ...message, id: pendingId, sequence: message.sequence + 0.5, client_mutation_id: null, role: "assistant", content: "", status: "streaming", model: null, token_count: 0, search_sources: [] });
    state.setBusy(true);
    try {
      const answer = await api.respond(activeId, message.id, searchEnabled);
      state.removeMessage(activeId, pendingId);
      state.upsertMessage(activeId, { ...message, status: "complete", updated_at: now() });
      state.upsertMessage(activeId, answer);
    } catch (cause) {
      state.removeMessage(activeId, pendingId);
      state.upsertMessage(activeId, { ...message, status: "error", updated_at: now() });
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
    try { await api.deleteMessage(activeId, message.id); }
    catch (cause) { state.setMessages(activeId, before); state.setError(cause instanceof Error ? cause.message : "Could not delete message."); }
  }

  return <main className={`app-shell ${sidebarCollapsed ? "sidebar-is-collapsed" : ""}`}><Sidebar conversations={conversations} activeId={activeId} open={sidebarOpen} collapsed={sidebarCollapsed} onNew={() => void newChat()} onSelect={state.setActiveId} onArchive={(id) => void archive(id)} onSettings={() => setProviderOpen(true)} onClose={() => state.setSidebarOpen(false)} onCollapse={() => setSidebarCollapsed(true)} userName={state.session?.user.display_name ?? "Account"} onSignOut={signOut} /><section className="chat-shell"><header className="chat-header"><button className="icon-button desktop-reopen" aria-label="Open navigation" onClick={() => setSidebarCollapsed(false)}><PanelLeftOpen size={19} /></button><button className="icon-button mobile-only" aria-label="Open navigation" onClick={() => state.setSidebarOpen(true)}><Menu size={20} /></button><div className="header-title"><span>{activeConversation?.title ?? "malim_chat"}</span></div>{activeConversation && <ModelSelector conversation={activeConversation} providers={providers} open={modelMenuOpen} busy={modelChanging} onToggle={() => setModelMenuOpen(!modelMenuOpen)} onChange={changeModel} />}</header>{error && <div className="notice"><CircleAlert size={16} /><span>{error}</span><button className="icon-button" aria-label="Dismiss" onClick={() => state.setError(null)}><X size={16} /></button></div>}<ConversationView conversation={activeConversation} messages={activeId ? messages[activeId] ?? [] : []} providers={providers} searchEnabled={searchEnabled} busy={busy} onToggleSearch={() => setSearchEnabled(!searchEnabled)} onSend={(content) => activeId ? sendMessage(activeId, content, searchEnabled) : Promise.resolve()} onEdit={editMessage} onDelete={deleteMessage} onRetry={retryMessage} onLookup={setLookupWord} onCompact={async () => { if (!activeId) return; try { await api.compact(activeId); state.setError("Conversation context was compacted."); } catch (cause) { state.setError(cause instanceof Error ? cause.message : "Compaction failed."); } }} /></section>{providerOpen && <ProviderDialog providers={providers} onClose={() => setProviderOpen(false)} onChanged={async () => { state.setProviders(await api.providers()); }} />}{lookupWord && <DictionaryDialog word={lookupWord} onClose={() => setLookupWord(null)} />}</main>;
}

function Sidebar({ conversations, activeId, open, collapsed, onNew, onSelect, onArchive, onSettings, onClose, onCollapse, userName, onSignOut }: { conversations: Conversation[]; activeId: string | null; open: boolean; collapsed: boolean; onNew: () => void; onSelect: (id: string) => void; onArchive: (id: string) => void; onSettings: () => void; onClose: () => void; onCollapse: () => void; userName: string; onSignOut: () => void }) {
  return <aside className={`sidebar ${open ? "open" : ""} ${collapsed ? "collapsed" : ""}`}><div className="sidebar-top"><div className="wordmark"><Bot size={20} /><span>malim_chat</span></div><button className="icon-button desktop-only" aria-label="Collapse navigation" onClick={onCollapse}><PanelLeftClose size={19} /></button><button className="icon-button mobile-only" aria-label="Close navigation" onClick={onClose}><X size={19} /></button></div><button className="new-chat" type="button" onClick={onNew}><SquarePen size={17} />New chat</button><nav aria-label="Conversation history" className="history"><p>Conversations</p>{conversations.map((conversation) => <div className={`conversation-row ${conversation.id === activeId ? "selected" : ""}`} key={conversation.id}><button type="button" onClick={() => onSelect(conversation.id)}><MessageSquare size={16} /><span>{conversation.title}</span></button><button type="button" className="row-action" aria-label={`Archive ${conversation.title}`} onClick={() => onArchive(conversation.id)}><Archive size={15} /></button></div>)}</nav><div className="sidebar-bottom"><button className="settings-button" type="button" onClick={onSettings}><Settings2 size={17} />Providers & settings</button><div className="account-row"><div className="avatar">{userName.slice(0, 1).toUpperCase()}</div><span>{userName}</span><button type="button" className="row-action" aria-label="Sign out" title="Sign out" onClick={onSignOut}><LogOut size={17} /></button></div></div></aside>;
}

function ModelSelector({ conversation, providers, open, busy, onToggle, onChange }: { conversation: Conversation; providers: Provider[]; open: boolean; busy: boolean; onToggle: () => void; onChange: (providerId: string, model: string) => Promise<void> }) {
  const activeProvider = providers.find((provider) => provider.id === conversation.model_provider_id);
  const [providerId, setProviderId] = useState(conversation.model_provider_id ?? providers[0]?.id ?? "");
  const [model, setModel] = useState(conversation.model ?? activeProvider?.default_model ?? "");
  useEffect(() => { setProviderId(conversation.model_provider_id ?? providers[0]?.id ?? ""); setModel(conversation.model ?? activeProvider?.default_model ?? ""); }, [conversation.id, conversation.model_provider_id, conversation.model, activeProvider?.default_model, providers]);
  return <div className="model-selector"><button type="button" className="model-button" aria-expanded={open} onClick={onToggle}><span>{model || "Choose model"}</span><ChevronsUpDown size={15} /></button>{open && <div className="model-popover"><label>Provider<select value={providerId} onChange={(event) => { const next = providers.find((provider) => provider.id === event.target.value); setProviderId(event.target.value); if (next) setModel(next.default_model); }}>{providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}</select></label><label>Model<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Model name" /></label><button type="button" className="primary small" disabled={!providerId || !model.trim() || busy} onClick={() => void onChange(providerId, model)}>{busy && <LoaderCircle className="spin" size={14} />}Use this model</button></div>}</div>;
}

function ConversationView({ conversation, messages, providers, searchEnabled, busy, onToggleSearch, onSend, onEdit, onDelete, onRetry, onLookup, onCompact }: { conversation: Conversation | null; messages: Message[]; providers: Provider[]; searchEnabled: boolean; busy: boolean; onToggleSearch: () => void; onSend: (content: string) => Promise<void>; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string) => void; onCompact: () => Promise<void> }) {
  const [input, setInput] = useState("");
  const endRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length, busy]);
  if (!conversation) return <section className="empty-state"><div className="empty-logo"><Bot size={34} /></div><h1>How can I help you today?</h1><p>Start a new conversation to work with your configured AI providers.</p></section>;
  const usage = Math.min(100, Math.round((conversation.context_tokens / conversation.context_window) * 100));
  const hasActiveProvider = providers.some((provider) => provider.id === conversation.model_provider_id);
  function send() { const text = input.trim(); if (!text || busy || !hasActiveProvider) return; setInput(""); void onSend(text); }
  function submit(event: FormEvent) { event.preventDefault(); send(); }
  return <><section className="message-list">{messages.map((message) => <MessageBubble key={message.id} message={message} onEdit={onEdit} onDelete={onDelete} onRetry={onRetry} onLookup={onLookup} />)}<div ref={endRef} /></section><footer className="composer-wrap"><div className="context-meter"><span>Context {conversation.context_tokens.toLocaleString()} / {conversation.context_window.toLocaleString()} tokens</span><div><i style={{ width: `${usage}%` }} /></div><button type="button" onClick={() => void onCompact()} title="Compact previous context">Compact</button></div><form className="composer" onSubmit={submit}><textarea value={input} onChange={(event) => setInput(event.target.value)} placeholder={hasActiveProvider ? "Message malim_chat" : "Choose a model from the conversation header"} disabled={!hasActiveProvider || busy} rows={1} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); send(); } }} /><div className="composer-controls"><button type="button" className={searchEnabled ? "search-chip enabled" : "search-chip"} aria-pressed={searchEnabled} onClick={onToggleSearch}><Search size={14} />{searchEnabled ? "Web search on" : "Web search off"}</button><button type="submit" className="send-button" aria-label="Send message" disabled={!input.trim() || busy || !hasActiveProvider}>{busy ? <LoaderCircle className="spin" size={18} /> : <Check size={18} />}</button></div></form><p className="disclaimer">AI responses may be inaccurate. Review important information.</p></footer></>;
}

function MessageBubble({ message, onEdit, onDelete, onRetry, onLookup }: { message: Message; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [selection, setSelection] = useState("");
  function captureSelection() { const selected = window.getSelection()?.toString().trim() ?? ""; setSelection(selected.length <= 120 ? selected : ""); }
  return <article className={`message ${message.role === "user" ? "user" : "assistant"} ${message.status === "error" ? "message-error" : ""}`}><div className="message-avatar">{message.role === "user" ? "You" : <Bot size={17} />}</div><div className="message-body" onMouseUp={captureSelection} onTouchEnd={captureSelection}>{editing ? <div className="edit-box"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} /><button className="primary small" type="button" onClick={() => { void onEdit(message, draft); setEditing(false); }}>Save</button><button className="text-button small" type="button" onClick={() => { setDraft(message.content); setEditing(false); }}>Cancel</button></div> : message.status === "streaming" ? <span className="typing"><i /><i /><i /></span> : <div className="message-content">{message.content}</div>}{message.status === "error" && message.role === "user" && <button type="button" className="retry-button" onClick={() => void onRetry(message)}>Retry response</button>}{message.search_sources?.length > 0 && <div className="sources">{message.search_sources.slice(0, 3).map((source) => <a href={source.url} target="_blank" rel="noreferrer" key={source.url}>{source.title}</a>)}</div>}<div className="message-tools"><button type="button" title="Copy" onClick={() => void navigator.clipboard.writeText(message.content)}><Copy size={14} /></button>{selection && <button type="button" title="Look up selected text" onClick={() => onLookup(selection)}><Languages size={14} /></button>}{message.role === "user" && !message.optimistic && <button type="button" title="Edit" onClick={() => setEditing(true)}><Edit3 size={14} /></button>}<button type="button" title="Delete" onClick={() => void onDelete(message)}><Trash2 size={14} /></button></div></div></article>;
}

function ProviderDialog({ providers, onClose, onChanged }: { providers: Provider[]; onClose: () => void; onChanged: () => Promise<void> }) {
  const [name, setName] = useState(""); const [kind, setKind] = useState("openai_compatible"); const [baseUrl, setBaseUrl] = useState("https://api.openai.com"); const [key, setKey] = useState(""); const [model, setModel] = useState(""); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) { event.preventDefault(); setBusy(true); setError(null); try { await api.createProvider({ name, kind, base_url: baseUrl, api_key: key, default_model: model }); await onChanged(); setName(""); setKey(""); setModel(""); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save provider."); } finally { setBusy(false); } }
  return <div className="modal-backdrop" role="presentation"><section className="modal" role="dialog" aria-modal="true" aria-labelledby="provider-title"><header><div><h2 id="provider-title">Providers</h2><p>Credentials are encrypted on the server and never returned to this device.</p></div><button className="icon-button" aria-label="Close" onClick={onClose}><X size={20} /></button></header><div className="provider-list">{providers.length === 0 ? <p className="muted">No providers configured.</p> : providers.map((provider) => <div className="provider-card" key={provider.id}><div><strong>{provider.name}</strong><span>{provider.default_model} · {provider.kind === "anthropic" ? "Anthropic" : "OpenAI-compatible"}</span></div><button className="icon-button danger" aria-label={`Delete ${provider.name}`} onClick={async () => { await api.deleteProvider(provider.id); await onChanged(); }}><Trash2 size={17} /></button></div>)}</div><form className="provider-form" onSubmit={submit}><h3>Add provider</h3><label>Name<input required value={name} onChange={(event) => setName(event.target.value)} placeholder="OpenAI, DeepSeek, Qwen..." /></label><label>API format<select value={kind} onChange={(event) => setKind(event.target.value)}><option value="openai_compatible">OpenAI-compatible</option><option value="anthropic">Anthropic Messages API</option></select></label><label>Base URL<input type="url" required value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" required value={key} onChange={(event) => setKey(event.target.value)} /></label><label>Default model<input required value={model} onChange={(event) => setModel(event.target.value)} placeholder="gpt-4o-mini" /></label>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></form></section></div>;
}

function DictionaryDialog({ word, onClose }: { word: string; onClose: () => void }) {
  const [dictionary, setDictionary] = useState<"russian_en" | "german_en" | "english_zh">(/[\u0400-\u04FF]/.test(word) ? "russian_en" : "english_zh"); const [data, setData] = useState<DictionaryResponse | null>(null); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => { let live = true; setBusy(true); setError(null); setData(null); void api.dictionary(word, dictionary).then((response) => { if (live) setData(response); }).catch((cause) => { if (live) setError(cause instanceof Error ? cause.message : "Dictionary lookup failed."); }).finally(() => { if (live) setBusy(false); }); return () => { live = false; }; }, [word, dictionary]);
  return <div className="modal-backdrop" role="presentation"><section className="modal dictionary-modal" role="dialog" aria-modal="true" aria-labelledby="dictionary-title"><header><div><h2 id="dictionary-title">{word}</h2><p>Local dictionary lookup</p></div><button className="icon-button" aria-label="Close" onClick={onClose}><X size={20} /></button></header><div className="dictionary-tabs"><button className={dictionary === "english_zh" ? "active" : ""} onClick={() => setDictionary("english_zh")}>English - Chinese</button><button className={dictionary === "german_en" ? "active" : ""} onClick={() => setDictionary("german_en")}>German - English</button><button className={dictionary === "russian_en" ? "active" : ""} onClick={() => setDictionary("russian_en")}>Russian - English</button></div>{busy && <p className="dictionary-loading"><LoaderCircle className="spin" size={17} />Looking up local dictionary...</p>}{error && <p className="form-error">{error}</p>}{data && data.entries.length === 0 && <p className="muted">No local entry found for this selection.</p>}<div className="dictionary-results">{data?.entries.map((entry, index) => <article className="dictionary-entry" key={`${entry.headword}-${index}`}><div className="dictionary-head"><h3>{entry.headword}</h3>{entry.pronunciation && <span>/{entry.pronunciation}/</span>}</div>{entry.translations.length > 0 && <section><h4>Chinese</h4><ul>{entry.translations.map((value) => <li key={value}>{value}</li>)}</ul></section>}{entry.definitions.length > 0 && <section><h4>Definitions</h4><ul>{entry.definitions.map((value) => <li key={value}>{value}</li>)}</ul></section>}{entry.forms.length > 0 && <section><h4>Forms</h4><div className="dictionary-pills">{entry.forms.map((value) => <span key={value}>{value}</span>)}</div></section>}{entry.examples.length > 0 && <section><h4>Examples</h4><ul>{entry.examples.map((value) => <li key={value}>{value}</li>)}</ul></section>}{entry.detail !== null && <details className="dictionary-detail"><summary>Entry detail</summary><pre>{typeof entry.detail === "string" ? entry.detail : JSON.stringify(entry.detail, null, 2)}</pre></details>}{entry.labels.length > 0 && <div className="dictionary-pills muted-pills">{entry.labels.map((value) => <span key={value}>{value}</span>)}</div>}</article>)}</div></section></div>;
}
