export type ProviderKind = "openai_compatible" | "anthropic";
export type Role = "user" | "assistant" | "system" | "tool" | "summary";
export interface User { id: string; email: string; display_name: string; created_at: string }
export interface ProviderModel { id: string; provider_id: string; group_name: string; model: string; sort_order: number; created_at: string; updated_at: string }
export interface Provider { id: string; name: string; kind: ProviderKind; base_url: string; default_model: string; models: ProviderModel[]; created_at: string; updated_at: string }
export interface Conversation { id: string; title: string; model_provider_id: string | null; model: string | null; context_window: number; context_tokens: number; revision: number; created_at: string; updated_at: string }
export interface Message { id: string; conversation_id: string; sequence: number; client_mutation_id: string | null; role: Role; content: string; content_format: string; status: "pending" | "streaming" | "complete" | "error"; model: string | null; token_count: number; search_sources: SearchResult[]; edited_at: string | null; created_at: string; updated_at: string; optimistic?: boolean }
export interface SearchResult { title: string; url: string; content: string; engine: string }
export interface Page<T> { items: T[]; next_cursor: string | null }
export interface Session { access_token: string; refresh_token: string; user: User }
export interface DictionaryEntry { headword: string; pronunciation: string; definitions: string[]; translations: string[]; forms: string[]; labels: string[]; examples: string[]; detail: unknown }
export interface DictionaryResponse { word: string; dictionary: "russian_en" | "german_en" | "english_zh"; entries: DictionaryEntry[] }
