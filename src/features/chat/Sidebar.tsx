import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Bot, ChevronDown, Edit3, LoaderCircle, LogOut, MessageSquare, PanelLeftClose, Settings2, SquarePen, Star, X } from "lucide-react";
import type { Conversation } from "../../types";
import { useFlip } from "../../hooks/use-flip";

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

export function Sidebar({ conversations, activeId, open, collapsed, hasMore, loadingMore, onLoadMore, onNew, onSelect, onRename, onToggleFavorite, onSettings, onClose, onCollapse, userName, onSignOut }: { conversations: Conversation[]; activeId: string | null; open: boolean; collapsed: boolean; hasMore: boolean; loadingMore: boolean; onLoadMore: () => Promise<void>; onNew: () => void; onSelect: (id: string) => void; onRename: (id: string, title: string) => Promise<void>; onToggleFavorite: (id: string) => void; onSettings: () => void; onClose: () => void; onCollapse: () => void; userName: string; onSignOut: () => void }) {
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
