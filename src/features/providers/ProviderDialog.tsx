import { FormEvent, useState } from "react";
import { LoaderCircle, Trash2, X } from "lucide-react";
import { api } from "../../api";
import type { Provider, ProviderKind } from "../../types";

type ModelDraft = { key: string; id?: string; group_name: string; model: string; kind: ProviderKind; context_window: number; supports_images: boolean; chain_context: boolean; hosted_tools: boolean };

const KIND_OPTIONS: [ProviderKind, string][] = [["openai_compatible", "OpenAI Chat"], ["openai_responses", "OpenAI Responses"], ["anthropic", "Anthropic"]];

function KindToggle({ value, label, onChange }: { value: ProviderKind; label: string; onChange: (value: ProviderKind) => void }) {
  return <div className="kind-toggle" role="radiogroup" aria-label={label}>{KIND_OPTIONS.map(([option, text]) => <button type="button" key={option} role="radio" aria-checked={value === option} className={value === option ? "selected" : ""} onClick={() => onChange(option)}>{text}</button>)}</div>;
}

/**
 * The whole provider page is one form: credentials and every model row are edited
 * locally and written together, so nothing can be left half-saved.
 */
function ProviderSettings({ provider, onChanged, onError }: { provider: Provider; onChanged: () => Promise<void>; onError: (value: string | null) => void }) {
  const [name, setName] = useState(provider.name);
  const [kind, setKind] = useState<ProviderKind>(provider.kind);
  const [baseUrl, setBaseUrl] = useState(provider.base_url);
  const [key, setKey] = useState("");
  const [drafts, setDrafts] = useState<ModelDraft[]>(provider.models.map((item) => ({ ...item, key: item.id })));
  const [removed, setRemoved] = useState<string[]>([]);
  const [adding, setAdding] = useState({ group_name: "General", model: "", kind: provider.kind, context_window: 128000, supports_images: false, chain_context: true, hosted_tools: true });
  const [busy, setBusy] = useState(false);
  const originals = new Map(provider.models.map((item) => [item.id, item]));
  const modelChanged = (item: ModelDraft) => {
    const before = item.id ? originals.get(item.id) : undefined;
    return !before || before.group_name !== item.group_name || before.model !== item.model || before.kind !== item.kind || before.context_window !== item.context_window || before.supports_images !== item.supports_images || before.chain_context !== item.chain_context || before.hosted_tools !== item.hosted_tools;
  };
  const dirty = name !== provider.name || kind !== provider.kind || baseUrl !== provider.base_url || key.trim() !== "" || removed.length > 0 || drafts.some(modelChanged);
  function update(key: string, patch: Partial<ModelDraft>) { setDrafts((current) => current.map((item) => item.key === key ? { ...item, ...patch } : item)); }
  function addModel() {
    if (!adding.model.trim()) return;
    setDrafts((current) => [...current, { ...adding, key: `new-${current.length}-${Date.now()}` }]);
    setAdding({ ...adding, model: "" });
  }
  function discard() {
    setDrafts(provider.models.map((item) => ({ ...item, key: item.id })));
    setRemoved([]);
    setName(provider.name);
    setKind(provider.kind);
    setBaseUrl(provider.base_url);
    setKey("");
    onError(null);
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!dirty || busy) return;
    setBusy(true);
    onError(null);
    try {
      if (name !== provider.name || kind !== provider.kind || baseUrl !== provider.base_url || key.trim()) await api.updateProvider(provider.id, { name: name.trim(), kind, base_url: baseUrl.trim(), api_key: key.trim() || undefined });
      for (const item of drafts) {
        const payload = { group_name: item.group_name.trim() || "General", model: item.model.trim(), kind: item.kind, context_window: item.context_window, supports_images: item.supports_images, chain_context: item.chain_context, hosted_tools: item.hosted_tools };
        if (!item.id) await api.createProviderModel(provider.id, payload);
        else if (modelChanged(item)) await api.updateProviderModel(provider.id, item.id, payload);
      }
      for (const id of removed) await api.deleteProviderModel(provider.id, id);
      setKey("");
      await onChanged();
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : "Could not save the provider.");
    } finally {
      setBusy(false);
    }
  }
  return <form className="provider-form" onSubmit={save}><h3>Edit provider <small>{provider.name}</small></h3><label>Provider API format<KindToggle value={kind} label="Provider API format" onChange={setKind} /></label><label>Name<input value={name} onChange={(event) => setName(event.target.value)} /></label><label>Base URL<input type="url" value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" value={key} onChange={(event) => setKey(event.target.value)} placeholder="Leave blank to keep the current key" /></label><section className="provider-models"><h3>Configured models <small>saved together with the credentials above</small></h3>{drafts.map((item) => <div className="model-config-row" key={item.key}><input value={item.group_name} aria-label="Model group" onChange={(event) => update(item.key, { group_name: event.target.value })} /><input value={item.model} aria-label="Model name" onChange={(event) => update(item.key, { model: event.target.value })} /><KindToggle value={item.kind} label={`${item.model || "Model"} API format`} onChange={(value) => update(item.key, { kind: value })} /><input value={item.context_window} type="number" min="4096" max="2000000" aria-label="Maximum context tokens" onChange={(event) => update(item.key, { context_window: Number(event.target.value) })} /><label className="model-toggle" title="This model can analyze images"><input type="checkbox" checked={item.supports_images} onChange={(event) => update(item.key, { supports_images: event.target.checked })} />Vision</label>{item.kind === "openai_responses" && <label className="model-toggle" title="Reuse the conversation the provider already stores instead of resending it every turn"><input type="checkbox" checked={item.chain_context} onChange={(event) => update(item.key, { chain_context: event.target.checked })} />Reuse context</label>}{item.kind !== "openai_compatible" && <label className="model-toggle" title="Let the provider run its own search, page reading and code execution. Turn off to fall back to our SearXNG ReAct loop, or when the gateway rejects the tools."><input type="checkbox" checked={item.hosted_tools} onChange={(event) => update(item.key, { hosted_tools: event.target.checked })} />Hosted tools</label>}<button type="button" className="icon-button danger" aria-label={`Remove ${item.model}`} onClick={() => { setDrafts((current) => current.filter((row) => row.key !== item.key)); if (item.id) setRemoved((current) => [...current, item.id as string]); }}><Trash2 size={15} /></button></div>)}<div className="add-model"><input value={adding.group_name} aria-label="New model group" onChange={(event) => setAdding({ ...adding, group_name: event.target.value })} placeholder="Group" /><input value={adding.model} aria-label="New model name" onChange={(event) => setAdding({ ...adding, model: event.target.value })} placeholder="Model name" /><KindToggle value={adding.kind} label="New model API format" onChange={(value) => setAdding({ ...adding, kind: value })} /><input value={adding.context_window} type="number" min="4096" max="2000000" aria-label="New model context tokens" onChange={(event) => setAdding({ ...adding, context_window: Number(event.target.value) })} /><label className="model-toggle"><input type="checkbox" checked={adding.supports_images} onChange={(event) => setAdding({ ...adding, supports_images: event.target.checked })} />Vision</label>{adding.kind === "openai_responses" && <label className="model-toggle"><input type="checkbox" checked={adding.chain_context} onChange={(event) => setAdding({ ...adding, chain_context: event.target.checked })} />Reuse context</label>}{adding.kind !== "openai_compatible" && <label className="model-toggle" title="Let the provider run its own search, page reading and code execution. Turn off to fall back to our SearXNG ReAct loop, or when the gateway rejects the tools."><input type="checkbox" checked={adding.hosted_tools} onChange={(event) => setAdding({ ...adding, hosted_tools: event.target.checked })} />Hosted tools</label>}<button type="button" className="text-button small" onClick={addModel}>Add model</button></div></section><div className="provider-save">{dirty && <button type="button" className="text-button" onClick={discard}>Discard changes</button>}<button className="primary" disabled={busy || !dirty}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></div></form>;
}

export function ProviderDialog({ providers, onClose, onChanged: publish }: { providers: Provider[]; onClose: () => void; onChanged: () => Promise<void> }) {
  // Bumping the revision remounts the settings form so it re-seeds from saved values.
  const [revision, setRevision] = useState(0);
  const onChanged = async () => { await publish(); setRevision((value) => value + 1); };
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ProviderKind>("openai_compatible");
  const [baseUrl, setBaseUrl] = useState("https://api.openai.com/v1");
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(providers[0]?.id ?? null);
  function setProviderKind(next: ProviderKind) {
    setKind(next);
    if (next === "anthropic") setBaseUrl("https://api.anthropic.com");
    else if (next === "openai_responses") setBaseUrl("https://api.openai.com/v1");
    else setBaseUrl("https://api.openai.com/v1");
  }
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true); setError(null);
    try { await api.createProvider({ name, kind, base_url: baseUrl, api_key: key }); await onChanged(); setName(""); setKey(""); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save provider."); }
    finally { setBusy(false); }
  }
  const editing = providers.find((provider) => provider.id === editingId) ?? null;
  return <div className="modal-backdrop" role="presentation"><section className="modal provider-modal" role="dialog" aria-modal="true" aria-labelledby="provider-title"><header><div><h2 id="provider-title">Providers</h2><p>Credentials are encrypted on the server and never returned to this device.</p></div><button className="icon-button" aria-label="Close" onClick={onClose}><X size={20} /></button></header><div className="provider-list">{providers.length === 0 ? <p className="muted">No providers configured.</p> : providers.map((provider) => <div className={`provider-card ${provider.id === editingId ? "selected" : ""}`} key={provider.id}><button type="button" className="provider-card-main" onClick={() => setEditingId(provider.id)}><span className="provider-avatar">{provider.name.slice(0, 1).toUpperCase()}</span><span className="provider-copy"><strong>{provider.name}</strong><span>{provider.base_url} · {provider.models.length} models</span></span></button><button className="icon-button danger" aria-label={`Delete ${provider.name}`} onClick={async () => { await api.deleteProvider(provider.id); if (editingId === provider.id) setEditingId(null); await onChanged(); }}><Trash2 size={17} /></button></div>)}</div>{editing && <ProviderSettings key={`${editing.id}-${revision}`} provider={editing} onChanged={onChanged} onError={setError} />}<form className="provider-form" onSubmit={submit}><h3>Add provider <small>credentials only — models are configured after saving</small></h3><label>Provider API format<div className="kind-toggle" role="radiogroup" aria-label="Provider API format"><button type="button" role="radio" aria-checked={kind === "openai_compatible"} className={kind === "openai_compatible" ? "selected" : ""} onClick={() => setProviderKind("openai_compatible")}>OpenAI-compatible</button><button type="button" role="radio" aria-checked={kind === "openai_responses"} className={kind === "openai_responses" ? "selected" : ""} onClick={() => setProviderKind("openai_responses")}>OpenAI Responses</button><button type="button" role="radio" aria-checked={kind === "anthropic"} className={kind === "anthropic" ? "selected" : ""} onClick={() => setProviderKind("anthropic")}>Anthropic</button></div></label><label>Name<input required value={name} onChange={(event) => setName(event.target.value)} placeholder={kind === "anthropic" ? "Anthropic" : "OpenAI, DeepSeek, Qwen..."} /></label><label>Base URL<input type="url" required value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} /></label><label>API key<input type="password" required value={key} onChange={(event) => setKey(event.target.value)} /></label><p className="muted">Both API roots and versioned `/v1` base URLs are accepted. Add models below after saving.</p>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}Save provider</button></form></section></div>;
}
