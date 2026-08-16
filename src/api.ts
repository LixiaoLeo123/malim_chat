import type { Conversation, DictionaryResponse, Message, Page, Provider, ProviderModel, SearchResult, Session, User } from "./types";

const apiBase = import.meta.env.VITE_API_URL ?? "http://your-server.example/malim_chat_api";
let session: Session | null = null;
let refreshInFlight: Promise<Session | null> | null = null;
let sessionListener: ((value: Session | null) => void) | null = null;

export function setSession(value: Session | null) {
  session = value;
  if (typeof window !== "undefined") {
    if (value) localStorage.setItem("malim-session", JSON.stringify(value));
    else localStorage.removeItem("malim-session");
  }
  sessionListener?.(value);
}
export function subscribeSession(listener: (value: Session | null) => void) {
  sessionListener = listener;
  return () => { if (sessionListener === listener) sessionListener = null; };
}
export function getSession() { return session; }

async function refreshSession() {
  if (!session) return null;
  if (!refreshInFlight) {
    const refreshToken = session.refresh_token;
    refreshInFlight = fetch(`${apiBase}/v1/auth/refresh`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ refresh_token: refreshToken })
    }).then(async (response) => {
      if (!response.ok) { setSession(null); return null; }
      const updated = await response.json() as Session;
      setSession(updated);
      return updated;
    }).catch(() => { setSession(null); return null; }).finally(() => { refreshInFlight = null; });
  }
  return refreshInFlight;
}

async function request<T>(path: string, init: RequestInit = {}, retry = true): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("Content-Type", "application/json");
  if (session) headers.set("Authorization", `Bearer ${session.access_token}`);
  const response = await fetch(`${apiBase}${path}`, { ...init, headers });
  if (response.status === 401 && session && retry) {
    const refreshed = await refreshSession();
    if (refreshed) return request<T>(path, init, false);
  }
  if (!response.ok) {
    if (response.status === 401) setSession(null);
    const payload = await response.json().catch(() => null);
    throw new Error(payload?.error?.message ?? `Request failed (${response.status})`);
  }
  return response.status === 204 ? undefined as T : response.json();
}

export const api = {
  signup: (email: string, password: string, display_name: string) => request<Session>("/v1/auth/signup", { method: "POST", body: JSON.stringify({ email, password, display_name }) }, false),
  login: (email: string, password: string) => request<Session>("/v1/auth/login", { method: "POST", body: JSON.stringify({ email, password }) }, false),
  me: () => request<User>("/v1/me"),
  providers: () => request<Provider[]>("/v1/providers"),
  createProvider: (payload: { name: string; kind: string; base_url: string; api_key: string; default_model: string }) => request<Provider>("/v1/providers", { method: "POST", body: JSON.stringify(payload) }),
  deleteProvider: (id: string) => request<void>(`/v1/providers/${id}`, { method: "DELETE" }),
  createProviderModel: (providerId: string, payload: { group_name: string; model: string; sort_order?: number }) => request<ProviderModel>(`/v1/providers/${providerId}/models`, { method: "POST", body: JSON.stringify(payload) }),
  updateProviderModel: (providerId: string, modelId: string, payload: { group_name?: string; model?: string; sort_order?: number }) => request<ProviderModel>(`/v1/providers/${providerId}/models/${modelId}`, { method: "PATCH", body: JSON.stringify(payload) }),
  deleteProviderModel: (providerId: string, modelId: string) => request<void>(`/v1/providers/${providerId}/models/${modelId}`, { method: "DELETE" }),
  conversations: (cursor?: string) => request<Page<Conversation>>(`/v1/conversations?limit=50${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`),
  createConversation: (payload: { title?: string; provider_id?: string; model?: string }) => request<Conversation>("/v1/conversations", { method: "POST", body: JSON.stringify(payload) }),
  updateConversation: (id: string, payload: { title?: string; archived?: boolean; provider_id?: string; model?: string; context_window?: number }) => request<Conversation>(`/v1/conversations/${id}`, { method: "PATCH", body: JSON.stringify(payload) }),
  deleteConversation: (id: string) => request<void>(`/v1/conversations/${id}`, { method: "DELETE" }),
  messages: (id: string, cursor?: string) => request<Page<Message>>(`/v1/conversations/${id}/messages?limit=50${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`),
  createMessage: (conversationId: string, content: string, mutationId: string, search: boolean) => request<Message>(`/v1/conversations/${conversationId}/messages`, { method: "POST", body: JSON.stringify({ content, client_mutation_id: mutationId, search }) }),
  respond: (conversationId: string, messageId: string, search: boolean, options?: { temperature?: number; reasoning_effort?: string }) => request<Message>(`/v1/conversations/${conversationId}/respond`, { method: "POST", body: JSON.stringify({ message_id: messageId, search, ...options }) }),
  updateMessage: (conversationId: string, messageId: string, content: string) => request<Message>(`/v1/conversations/${conversationId}/messages/${messageId}`, { method: "PATCH", body: JSON.stringify({ content }) }),
  deleteMessage: (conversationId: string, messageId: string) => request<void>(`/v1/conversations/${conversationId}/messages/${messageId}`, { method: "DELETE" }),
  compact: (conversationId: string, force = true) => request<{ compacted: boolean }>(`/v1/conversations/${conversationId}/compact`, { method: "POST", body: JSON.stringify({ force }) }),
  dictionary: (word: string, dictionary: "russian_en" | "german_en" | "english_zh") => request<DictionaryResponse>(`/v1/dictionary?word=${encodeURIComponent(word)}&dictionary=${dictionary}`),
  search: (q: string) => request<SearchResult[]>(`/v1/search?q=${encodeURIComponent(q)}`)
};
