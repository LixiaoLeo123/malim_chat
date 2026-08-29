import { create } from "zustand";
import type { Conversation, Message, Provider, Session } from "./types";

type State = {
  session: Session | null; conversations: Conversation[]; providers: Provider[]; messages: Record<string, Message[]>; activeId: string | null; sidebarOpen: boolean; busy: boolean; error: string | null;
  setSession: (session: Session | null) => void; setConversations: (items: Conversation[]) => void; appendConversations: (items: Conversation[]) => void; setProviders: (items: Provider[]) => void; setMessages: (id: string, items: Message[]) => void; prependMessages: (id: string, items: Message[]) => void; upsertMessage: (id: string, message: Message) => void; removeMessage: (id: string, messageId: string) => void; setActiveId: (id: string | null) => void; setSidebarOpen: (open: boolean) => void; setBusy: (busy: boolean) => void; setError: (message: string | null) => void;
};
export const useAppStore = create<State>((set) => ({
  session: null, conversations: [], providers: [], messages: {}, activeId: null, sidebarOpen: false, busy: false, error: null,
  setSession: (session) => set({ session }),
  setConversations: (conversations) => set({ conversations }),
  appendConversations: (items) => set((state) => {
    const existing = new Set(state.conversations.map((conversation) => conversation.id));
    return { conversations: [...state.conversations, ...items.filter((conversation) => !existing.has(conversation.id))] };
  }),
  setProviders: (providers) => set({ providers }),
  setMessages: (id, items) => set((state) => ({ messages: { ...state.messages, [id]: items } })),
  prependMessages: (id, items) => set((state) => {
    const existing = state.messages[id] ?? [];
    const existingIds = new Set(existing.map((message) => message.id));
    return { messages: { ...state.messages, [id]: [...items.filter((message) => !existingIds.has(message.id)), ...existing] } };
  }),
  upsertMessage: (id, message) => set((state) => {
    const existing = state.messages[id] ?? [];
    let position = -1;
    for (let index = existing.length - 1; index >= 0; index--) {
      const candidate = existing[index];
      if (candidate.id === message.id || (message.client_mutation_id && candidate.client_mutation_id === message.client_mutation_id)) {
        position = index;
        break;
      }
    }
    if (position >= 0) {
      const items = existing.slice();
      items[position] = message;
      return { messages: { ...state.messages, [id]: items } };
    }
    const last = existing.at(-1);
    if (!last || message.sequence >= last.sequence) return { messages: { ...state.messages, [id]: [...existing, message] } };
    const items = [...existing, message];
    items.sort((left, right) => left.sequence - right.sequence);
    return { messages: { ...state.messages, [id]: items } };
  }),
  removeMessage: (id, messageId) => set((state) => ({ messages: { ...state.messages, [id]: (state.messages[id] ?? []).filter((message) => message.id !== messageId) } })),
  setActiveId: (activeId) => set({ activeId, sidebarOpen: false }), setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }), setBusy: (busy) => set({ busy }), setError: (error) => set({ error })
}));
