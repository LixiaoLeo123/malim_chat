import { useEffect, useMemo, useRef, useState } from "react";
import type { Message } from "../../types";

export function RightNavigator({ messages }: { messages: Message[] }) {
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
