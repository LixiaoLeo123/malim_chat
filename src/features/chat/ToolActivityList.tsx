import { useEffect, useRef, useState } from "react";
import { Bot, Check, CircleAlert, LoaderCircle } from "lucide-react";
import type { ToolActivity } from "../../types";
import { sourceHost } from "../../lib/app-utils";
import { MemoMarkdown } from "../../components/common/markdown";

export function ToolActivityList({ events, reasoning, streaming, theme, markdown }: { events?: ToolActivity[]; reasoning?: string; streaming?: boolean; theme: "light" | "dark"; markdown: boolean }) {
  const [open, setOpen] = useState(Boolean(streaming));
  const [height, setHeight] = useState(0);
  const contentRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (!streaming) setOpen(false); }, [streaming]);
  useEffect(() => { const node = contentRef.current; if (!node) return; const measure = () => setHeight(node.scrollHeight); measure(); const observer = new ResizeObserver(measure); observer.observe(node); return () => observer.disconnect(); }, [events, reasoning]);
  // Checkpoints saved before reasoning became a tool event carry bare `{delta, round}` rows.
  const steps = (events ?? []).flatMap((event, index) => {
    if (event.name) return [event];
    const delta = (event as ToolActivity & { delta?: string }).delta;
    if (!delta) return [];
    return [{ id: `reasoning-${event.round ?? index}`, name: "reasoning", status: "completed", round: event.round, detail: delta } satisfies ToolActivity];
  });
  if (!steps.length && !reasoning) return null;
  function label(event: ToolActivity) {
    const query = event.input?.query;
    const host = event.input?.url ? sourceHost(event.input.url) : "";
    if (event.status === "failed") return event.detail || `${event.name.replace(/_/g, " ")} failed`;
    if (event.name === "planning") return event.status === "running" ? "Planning research" : "Research step planned";
    if (event.name === "web_search") {
      if (event.status === "running") return query ? `Searching: ${query}` : "Searching the web";
      return event.source_count !== undefined ? `Search complete: ${event.source_count} sources` : "Search complete";
    }
    if (event.name === "open_web_page") return event.status === "running" ? `Reading: ${host || "source"}` : `Read: ${host || "source"}`;
    // The provider's own fetch and sandbox: their calls arrive with the result already
    // attached, so there is no input of ours to show.
    if (event.name === "web_fetch") return event.status === "running" ? "Reading a page" : `Read: ${sourceHost(event.detail ?? "") || "source"}`;
    if (event.name === "code_execution") return event.status === "running" ? "Running code" : event.detail ? `Code output: ${event.detail}` : "Code finished";
    if (event.name === "drafting") return event.status === "running" ? "Drafting answer" : "Answer drafted";
    if (event.name === "reasoning") return event.detail || "Reasoning";
    return event.status === "running" ? `${event.name.replace(/_/g, " ")} in progress` : `${event.name.replace(/_/g, " ")} complete`;
  }
  function copy(event: ToolActivity) {
    const text = label(event);
    if (event.name !== "reasoning" || !markdown) return <span className="tool-step-copy">{text}</span>;
    return <div className="tool-step-copy markdown-content"><MemoMarkdown content={text} theme={theme} /></div>;
  }
  return <section className={`agent-timeline ${open ? "open" : "collapsed"}`} aria-label="Agent process" aria-live="polite"><button type="button" className="agent-timeline-toggle" aria-expanded={open} onClick={() => setOpen(!open)}><span>{streaming ? "Agent working" : "Agent process"}</span><small>{open ? "Collapse" : "Expand"}</small></button><div className="agent-timeline-collapse" style={{ maxHeight: open ? height : 0 }}><div ref={contentRef} className="tool-activity">{steps.map((event) => <div key={event.id} className={`tool-step ${event.status}`}><span className="tool-step-icon">{event.name === "reasoning" ? <Bot size={12} /> : event.status === "running" ? <LoaderCircle className="spin" size={13} /> : event.status === "failed" ? <CircleAlert size={13} /> : <Check size={13} />}</span>{copy(event)}</div>)}{!steps.some((event) => event.name === "reasoning") && reasoning && <div className="tool-step completed"><span className="tool-step-icon"><Bot size={12} /></span>{markdown ? <div className="tool-step-copy markdown-content"><MemoMarkdown content={reasoning} theme={theme} /></div> : <span className="tool-step-copy">{reasoning}</span>}</div>}</div></div></section>;
}
