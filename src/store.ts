import { create } from "zustand";
import type { Conversation, Message, Provider, Session } from "./types";

type State = {
  session: Session | null; conversations: Conversation[]; providers: Provider[]; messages: Record<string, Message[]>; activeId: string | null; sidebarOpen: boolean; busy: boolean; error: string | null;
  setSession: (session: Session | null) => void; setConversations: (items: Conversation[]) => void; setProviders: (items: Provider[]) => void; setMessages: (id: string, items: Message[]) => void; upsertMessage: (id: string, message: Message) => void; removeMessage: (id: string, messageId: string) => void; setActiveId: (id: string | null) => void; setSidebarOpen: (open: boolean) => void; setBusy: (busy: boolean) => void; setError: (message: string | null) => void;
};
export const useAppStore = create<State>((set) => ({
  session: null, conversations: [], providers: [], messages: {}, activeId: null, sidebarOpen: false, busy: false, error: null,
  setSession: (session) => set({ session }), setConversations: (conversations) => set({ conversations }), setProviders: (providers) => set({ providers }),
  setMessages: (id, items) => set((state) => ({ messages: { ...state.messages, [id]: items } })),
  upsertMessage: (id, message) => set((state) => { const existing = state.messages[id] ?? []; const position = existing.findIndex((candidate) => candidate.id === message.id || (message.client_mutation_id && candidate.client_mutation_id === message.client_mutation_id)); const items = position >= 0 ? existing.map((candidate, index) => index === position ? message : candidate) : [...existing, message]; return { messages: { ...state.messages, [id]: items.sort((a, b) => a.sequence - b.sequence) } }; }),
  removeMessage: (id, messageId) => set((state) => ({ messages: { ...state.messages, [id]: (state.messages[id] ?? []).filter((message) => message.id !== messageId) } })),
  setActiveId: (activeId) => set({ activeId, sidebarOpen: false }), setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }), setBusy: (busy) => set({ busy }), setError: (error) => set({ error })
}));
