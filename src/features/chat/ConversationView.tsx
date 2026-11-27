import { ClipboardEvent, FormEvent, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Bot, Check, FileText, ImagePlus, LoaderCircle, Search, Square, X } from "lucide-react";
import type { Conversation, FileAttachment, GenerationSettings, Message, Provider } from "../../types";
import { api } from "../../api";
import { MemoMessageBubble } from "./MessageBubble";
import { GenerationButton } from "../settings/GenerationButton";

export function ConversationView({ conversation, messages, hasOlder, loadingOlder, onLoadOlder, providers, searchEnabled, busy, compacting, theme, lookupActive, streaming, onStop, generation, onGenerationChange, onToggleSearch, onSend, onEdit, onDelete, onRetry, onLookup, onCompact }: { conversation: Conversation | null; messages: Message[]; hasOlder: boolean; loadingOlder: boolean; onLoadOlder: (id: string) => Promise<void>; providers: Provider[]; searchEnabled: boolean; busy: boolean; compacting: boolean; theme: "light" | "dark"; lookupActive: boolean; streaming: boolean; onStop: () => void; generation: GenerationSettings; onGenerationChange: (value: GenerationSettings) => void; onToggleSearch: () => void; onSend: (content: string, images?: string[], files?: FileAttachment[]) => Promise<void>; onEdit: (message: Message, content: string) => Promise<void>; onDelete: (message: Message) => Promise<void>; onRetry: (message: Message) => Promise<void>; onLookup: (word: string, anchor: { x: number; y: number }) => void; onCompact: () => Promise<void> }) {
  const [input, setInput] = useState("");
  const [images, setImages] = useState<string[]>([]);
  // Documents go up to the provider's Files API as soon as they are chosen, so sending
  // the message only references their ids.
  const [files, setFiles] = useState<FileAttachment[]>([]);
  const [uploading, setUploading] = useState(0);
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
  useEffect(() => { setImages([]); setFiles([]); setUploading(0); setImageError(null); setDragging(false); }, [conversation?.id]);
  if (!conversation) return <section className="empty-state"><div className="empty-logo"><Bot size={34} /></div><h1>How can I help you today?</h1><p>Start a new conversation to work with your configured AI providers.</p></section>;
  const usage = Math.min(100, Math.round((conversation.context_tokens / conversation.context_window) * 100));
  const hasActiveProvider = providers.some((provider) => provider.id === conversation.model_provider_id && provider.models.some((item) => item.model === conversation.model));
  const activeModel = providers.find((item) => item.id === conversation.model_provider_id)?.models.find((item) => item.model === conversation.model);
  // Hosted dialects run tools on the provider's side, so there is nothing to toggle —
  // the search chip only applies when the ReAct loop does the searching.
  const nativeSearch = activeModel?.kind === "google_gemini" || activeModel?.kind === "deepseek" || activeModel?.kind === "openai_responses" || (activeModel?.kind === "anthropic" && activeModel.hosted_tools === true);
  function uploadFile(file: File, providerId: string) {
    if (images.length + files.length >= 8) { setImageError("A message can hold up to 8 attachments."); return; }
    setUploading((current) => current + 1);
    const reader = new FileReader();
    reader.onload = () => {
      const data = typeof reader.result === "string" ? reader.result.split(",")[1] : undefined;
      if (!data) { setUploading((current) => current - 1); setImageError("That file could not be read."); return; }
      void api.uploadProviderFile(providerId, { filename: file.name, media_type: file.type || "application/octet-stream", data })
        .then((attachment) => { setFiles((current) => current.length >= 8 ? current : [...current, attachment]); setImageError(null); })
        .catch((cause) => setImageError(cause instanceof Error ? cause.message : "The file could not be uploaded."))
        .finally(() => setUploading((current) => current - 1));
    };
    reader.onerror = () => { setUploading((current) => current - 1); setImageError("That file could not be read."); };
    reader.readAsDataURL(file);
  }
  function addFiles(list: FileList | File[] | null) {
    if (!list) return;
    for (const file of Array.from(list)) {
      const isImage = file.type.startsWith("image/");
      const isGoogleVideo = activeModel?.kind === "google_gemini" && file.type.startsWith("video/");
      if (!isImage && !isGoogleVideo) {
        // A document only travels by reference, through the DeepSeek Files API.
        if (activeModel?.kind !== "deepseek") { setImageError(activeModel?.kind === "google_gemini" ? "Choose a supported image or video file." : "Only image files can be attached to this model."); continue; }
        if (!conversation?.model_provider_id) { setImageError("Choose a DeepSeek provider before uploading files."); continue; }
        if (file.size > 32 * 1024 * 1024) { setImageError("Each file must be 32 MB or smaller."); continue; }
        uploadFile(file, conversation.model_provider_id);
        continue;
      }
      const maxBytes = isGoogleVideo ? 10 * 1024 * 1024 : 5 * 1024 * 1024;
      if (file.size > maxBytes) { setImageError(isGoogleVideo ? "Each video must be 10 MB or smaller." : "Each image must be 5 MB or smaller."); continue; }
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result !== "string") return;
        setImages((current) => {
          if (current.length >= 8) { setImageError("A message can hold up to 8 attachments."); return current; }
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
      const file = item.kind === "file" && (item.type.startsWith("image/") || (activeModel?.kind === "google_gemini" && item.type.startsWith("video/"))) ? item.getAsFile() : null;
      if (file) files.push(file);
    }
    if (files.length > 0) { event.preventDefault(); addFiles(files); }
  }
  function send() {
    const text = input.trim();
    if ((!text && images.length === 0 && files.length === 0) || busy || uploading > 0 || !hasActiveProvider) return;
    const payload = images;
    const payloadFiles = files;
    setInput("");
    setImages([]);
    setFiles([]);
    setImageError(null);
    void onSend(text, payload, payloadFiles);
  }
  function submit(event: FormEvent) { event.preventDefault(); send(); }
  return <><section ref={messageListRef} className={generation.stream ? "message-list stream-output" : "message-list"}>{hasOlder && <button className="load-older" type="button" disabled={loadingOlder} onClick={() => conversation && void onLoadOlder(conversation.id)}>{loadingOlder ? "Loading older..." : "Load older messages"}</button>}{messages.map((message, index) => <MemoMessageBubble key={message.id} message={message} markdownEnabled={generation.enable_markdown} theme={theme} isLast={index === messages.length - 1} lookupActive={lookupActive} onEdit={onEdit} onDelete={onDelete} onRetry={onRetry} onLookup={onLookup} />)}</section><footer className="composer-wrap"><div className="context-meter"><span>Context {conversation.context_tokens.toLocaleString()} / {conversation.context_window.toLocaleString()} tokens</span><div><i style={{ width: `${usage}%` }} /></div><button type="button" disabled={compacting} onClick={() => void onCompact()} title="Compact previous context">{compacting ? <><LoaderCircle className="spin" size={12} />Compacting</> : "Compact"}</button></div><form className={`composer ${dragging ? "dragging" : ""}`} onSubmit={submit} onDragOver={(event) => { event.preventDefault(); setDragging(true); }} onDragLeave={() => setDragging(false)} onDrop={(event) => { event.preventDefault(); setDragging(false); addFiles(event.dataTransfer.files); }}><input ref={fileInputRef} type="file" accept={activeModel?.kind === "deepseek" ? "image/*,.pdf,.doc,.docx,.ppt,.pptx,.txt,.md,.csv,.json,.xlsx,.xls,.epub,.mobi" : activeModel?.kind === "google_gemini" ? "image/*,video/*" : "image/*"} multiple hidden onChange={(event) => { addFiles(event.target.files); event.target.value = ""; }} />{(images.length > 0 || files.length > 0 || uploading > 0) && <div className="image-previews">{images.map((src, index) => <div className="image-preview" key={`${index}-${src.slice(0, 40)}`}>{src.startsWith("data:video/") ? <video src={src} muted controls /> : <img src={src} alt={`Attachment ${index + 1}`} />}<button type="button" aria-label={`Remove attachment ${index + 1}`} title="Remove attachment" onClick={() => setImages(images.filter((_, item) => item !== index))}><X size={13} /></button></div>)}{files.map((file, index) => <div className="image-preview file-preview" key={file.id}><FileText size={15} /><span title={file.name}>{file.name}</span><button type="button" aria-label={`Remove ${file.name}`} title="Remove attachment" onClick={() => setFiles(files.filter((_, item) => item !== index))}><X size={13} /></button></div>)}{uploading > 0 && <div className="image-preview file-preview uploading" aria-live="polite"><LoaderCircle className="spin" size={14} /><span>Uploading…</span></div>}</div>}{imageError && <p className="image-error">{imageError}</p>}<textarea ref={inputRef} value={input} onChange={(event) => setInput(event.target.value)} placeholder={hasActiveProvider ? "Message malim_chat" : "Choose a model from the conversation header"} disabled={!hasActiveProvider || busy || compacting} rows={1} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); send(); } }} onPaste={handlePaste} /><div className="composer-controls"><button type="button" className="attach-button" aria-label={activeModel?.kind === "deepseek" ? "Attach images or documents" : "Attach images"} title={activeModel?.kind === "deepseek" ? "Attach images or documents" : "Attach images"} onClick={() => fileInputRef.current?.click()} disabled={!hasActiveProvider || busy || compacting}><ImagePlus size={17} /></button>{!nativeSearch && <button type="button" className={searchEnabled ? "search-chip enabled" : "search-chip"} aria-pressed={searchEnabled} onClick={onToggleSearch}><Search size={14} />{searchEnabled ? "Web search on" : "Web search off"}</button>}<GenerationButton generation={generation} onChange={onGenerationChange} kind={activeModel?.kind} hosted={nativeSearch} /><button type={streaming ? "button" : "submit"} className={`send-button${streaming ? " stop" : ""}`} aria-label={streaming ? "Stop generating" : "Send message"} title={streaming ? "Stop generating" : "Send"} onClick={streaming ? (event) => { event.preventDefault(); onStop(); } : undefined} disabled={!streaming && ((!input.trim() && images.length === 0 && files.length === 0) || busy || uploading > 0 || compacting || !hasActiveProvider)}>{streaming ? <Square size={17} /> : busy ? <LoaderCircle className="spin" size={18} /> : <Check size={18} />}</button></div></form><p className="disclaimer">AI responses may be inaccurate. Review important information.</p></footer></>;
}
