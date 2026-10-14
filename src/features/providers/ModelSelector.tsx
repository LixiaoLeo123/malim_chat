import { useEffect, useState } from "react";
import { ChevronsUpDown, LoaderCircle } from "lucide-react";
import type { Conversation, Provider, ProviderModel } from "../../types";

export function ModelSelector({ conversation, providers, open, busy, onToggle, onChange }: { conversation: Conversation; providers: Provider[]; open: boolean; busy: boolean; onToggle: () => void; onChange: (providerId: string, model: string) => Promise<void> }) {
  const activeProvider = providers.find((provider) => provider.id === conversation.model_provider_id);
  const [providerId, setProviderId] = useState(conversation.model_provider_id ?? providers[0]?.id ?? "");
  const [model, setModel] = useState(conversation.model ?? activeProvider?.models[0]?.model ?? "");
  useEffect(() => { setProviderId(conversation.model_provider_id ?? providers[0]?.id ?? ""); setModel(conversation.model ?? activeProvider?.models[0]?.model ?? ""); }, [conversation.id, conversation.model_provider_id, conversation.model, activeProvider?.models, providers]);
  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const groups = (selectedProvider?.models ?? []).reduce<Record<string, ProviderModel[]>>((all, item) => { (all[item.group_name] ??= []).push(item); return all; }, {});
  return <div className="model-selector"><button type="button" className="model-button" aria-expanded={open} disabled={busy} onClick={onToggle}><span>{model || "Choose model"}</span>{busy ? <LoaderCircle className="spin" size={15} /> : <ChevronsUpDown size={15} />}</button>{open && <div className="model-popover"><div className="model-picker-heading">Provider</div><div className="provider-options">{providers.map((provider) => <button type="button" key={provider.id} disabled={busy} className={provider.id === providerId ? "selected" : ""} onClick={() => { setProviderId(provider.id); setModel(provider.models[0]?.model ?? ""); }}>{provider.name}</button>)}</div><div className="model-picker-heading">Model</div><div className="configured-models">{Object.entries(groups).map(([group, items]) => <section key={group}><h4>{group}</h4>{items.map((item) => <button type="button" key={item.id} disabled={busy} className={item.model === model ? "selected" : ""} onClick={() => { setModel(item.model); void onChange(providerId, item.model); }}>{item.model}</button>)}</section>)}</div></div>}</div>;
}
