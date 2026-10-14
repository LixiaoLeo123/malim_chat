import { useEffect, useRef, useState } from "react";
import { Settings2 } from "lucide-react";
import type { GenerationSettings, ProviderKind } from "../../types";

/** A numeric parameter whose empty state means "provider default". Keeps what the user
 * types (a controlled number value re-parses every keystroke and eats the decimal point),
 * but commits the parsed, clamped value on each change so the debounced PATCH never sends
 * an out-of-range number. */
function ParameterNumber({ label, value, min, max, step, placeholder, onCommit }: { label: string; value: number | null; min: number; max: number; step: number; placeholder: string; onCommit: (value: number | null) => void }) {
  const [text, setText] = useState(() => (value === null ? "" : String(value)));
  const committed = useRef(value);
  useEffect(() => { // re-seed only when the value changes underneath (conversation switch)
    if (value !== committed.current) { committed.current = value; setText(value === null ? "" : String(value)); }
  }, [value]);
  const publish = (raw: string, normalize: boolean) => {
    if (raw.trim() === "") { committed.current = null; onCommit(null); if (normalize) setText(""); return; }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return;
    let clamped = Math.min(max, Math.max(min, parsed));
    if (step >= 1) clamped = Math.round(clamped); // integer parameters only
    committed.current = clamped;
    onCommit(clamped);
    if (normalize) setText(String(clamped)); // on blur only, so typing stays uninterrupted
  };
  return <label><span>{label}</span><input type="number" inputMode="decimal" min={min} max={max} step={step} value={text} placeholder={placeholder} onChange={(event) => { setText(event.target.value); publish(event.target.value, false); }} onBlur={() => publish(text, true)} /></label>;
}

export function GenerationButton({ generation, onChange, kind, hosted }: { generation: GenerationSettings; onChange: (value: GenerationSettings) => void; kind?: ProviderKind; hosted: boolean }) {
  const [open, setOpen] = useState(false);
  const controlRef = useRef<HTMLDivElement>(null);
  // Responses stores the conversation on its own side, so there is no window to trim; and
  // wherever the provider runs its own tools there is no ReAct loop of ours to bound.
  const chained = kind === "openai_responses";
  // The parameters follow the dialect: Anthropic's temperature stops at 1, only it takes
  // top_k, and its effort rides the top-level `effort` field instead of the OpenAI keys.
  const isAnthropic = kind === "anthropic";
  const tempMax = isAnthropic ? 1 : 2;
  const temperature = Math.min(generation.temperature, tempMax);
  const effort = ["low", "medium", "high"] as const;
  useEffect(() => { const outside = (event: PointerEvent) => { if (!controlRef.current?.contains(event.target as Node)) setOpen(false); }; document.addEventListener("pointerdown", outside, true); return () => document.removeEventListener("pointerdown", outside, true); }, []);
  const context = generation.context_rounds ?? null; const tools = generation.tool_rounds ?? null;
  return <div ref={controlRef} className="generation-control"><button type="button" className="search-chip" onClick={() => setOpen(!open)} title="Generation parameters"><Settings2 size={14} />Parameters</button>{open && <div className="generation-popover"><label><span>Temperature <b>{temperature.toFixed(1)}</b></span><input type="range" min="0" max={tempMax} step="0.1" value={temperature} onChange={(event) => onChange({ ...generation, temperature: Number(event.target.value) })} /></label><label><span>{isAnthropic ? "Effort" : "Reasoning effort"} <b>{generation.reasoning_effort}</b></span><input type="range" min="0" max="2" step="1" value={effort.indexOf(generation.reasoning_effort as typeof effort[number])} onChange={(event) => onChange({ ...generation, reasoning_effort: effort[Number(event.target.value)] })} /><small>Low <i /> Medium <i /> High</small></label><ParameterNumber label="Top P" value={generation.top_p ?? null} min={0} max={1} step={0.05} placeholder="Default" onCommit={(top_p) => onChange({ ...generation, top_p })} />{isAnthropic && <ParameterNumber label="Top K" value={generation.top_k ?? null} min={1} max={1000} step={1} placeholder="Default" onCommit={(top_k) => onChange({ ...generation, top_k })} />}<ParameterNumber label="Max output tokens" value={generation.max_tokens ?? null} min={1} max={1000000} step={1} placeholder={isAnthropic ? "8192" : "Default"} onCommit={(max_tokens) => onChange({ ...generation, max_tokens })} />{hosted && <><ParameterNumber label="Max web searches" value={generation.web_search_uses ?? null} min={1} max={50} step={1} placeholder="5" onCommit={(web_search_uses) => onChange({ ...generation, web_search_uses })} /><ParameterNumber label="Max page fetches" value={generation.web_fetch_uses ?? null} min={1} max={50} step={1} placeholder="5" onCommit={(web_fetch_uses) => onChange({ ...generation, web_fetch_uses })} /></>}{!chained && <label><span>Recent context rounds <b>{context === null ? "All" : context}</b></span><input type="range" min="1" max="20" step="1" value={context === null ? 20 : context} onChange={(event) => { const value = Number(event.target.value); onChange({ ...generation, context_rounds: value >= 20 ? null : value }); }} /><small>1 <i /> All</small></label>}{!hosted && <label><span>Maximum ReAct rounds <b>{tools === null ? "Unlimited" : tools}</b></span><input type="range" min="1" max="20" step="1" value={tools === null ? 20 : tools} onChange={(event) => { const value = Number(event.target.value); onChange({ ...generation, tool_rounds: value >= 20 ? null : value }); }} /><small>1 <i /> Unlimited</small></label>}<label className="markdown-toggle"><span>Enable Markdown</span><input type="checkbox" checked={generation.enable_markdown} onChange={(event) => onChange({ ...generation, enable_markdown: event.target.checked })} /></label><label className="markdown-toggle"><span>Stream output</span><input type="checkbox" checked={generation.stream} onChange={(event) => onChange({ ...generation, stream: event.target.checked })} /></label></div>}</div>;
}
