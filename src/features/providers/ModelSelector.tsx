import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, ChevronLeft, ChevronsUpDown, Cpu, LoaderCircle } from "lucide-react";
import type { Conversation, Provider, ProviderModel } from "../../types";

/** Models of one provider, bucketed by their configured group, in first-seen order. */
function groupModels(models: ProviderModel[]) {
  const groups = new Map<string, ProviderModel[]>();
  for (const model of models) {
    const name = model.group_name.trim() || "General";
    groups.set(name, [...(groups.get(name) ?? []), model]);
  }
  return [...groups.entries()];
}

/**
 * Header model picker: one row per provider from Preferences. Hovering (or tapping) a
 * row opens that provider's models on the left, aligned with the row and grouped under
 * their group names. The main menu never moves.
 */
export function ModelSelector({ conversation, providers, open, busy, onToggle, onChange }: { conversation: Conversation; providers: Provider[]; open: boolean; busy: boolean; onToggle: () => void; onChange: (providerId: string, model: string) => Promise<void> }) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const flyoutRef = useRef<HTMLDivElement | null>(null);
  const currentProviderId = conversation.model_provider_id;
  const currentModel = conversation.model ?? "";
  const selectedProvider = providers.find((provider) => provider.id === currentProviderId);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [flyoutTop, setFlyoutTop] = useState(0);

  useEffect(() => {
    if (!open) { setHoveredId(null); return; }
    const close = (event: MouseEvent) => { if (containerRef.current && !containerRef.current.contains(event.target as Node)) onToggle(); };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open, onToggle]);

  function showProvider(id: string, row: HTMLElement) {
    const container = containerRef.current?.querySelector(".model-flyout-container");
    if (!container) return;
    // Line the flyout's first entry up with the hovered row (card padding + border = 9px).
    setFlyoutTop(row.getBoundingClientRect().top - container.getBoundingClientRect().top - 9);
    setHoveredId(id);
  }

  // Keep a tall flyout inside the viewport by sliding it up as far as needed.
  useLayoutEffect(() => {
    const flyout = flyoutRef.current;
    if (!flyout || getComputedStyle(flyout).position !== "absolute") return;
    const overflow = flyout.getBoundingClientRect().bottom - (window.innerHeight - 12);
    if (overflow > 0) setFlyoutTop((top) => Math.max(top - overflow, -(flyout.parentElement?.getBoundingClientRect().top ?? 0) + 12));
  }, [hoveredId, flyoutTop]);

  const hovered = providers.find((provider) => provider.id === hoveredId);

  return <div className="model-selector" ref={containerRef}>
    <button type="button" className="model-button" aria-expanded={open} disabled={busy} onClick={onToggle}>
      <Cpu className="model-trigger-icon" size={15} />
      <span className="model-trigger-name">{currentModel || "Choose model"}</span>
      {selectedProvider && <span className="model-trigger-badge">{selectedProvider.name}</span>}
      {busy ? <LoaderCircle className="spin model-trigger-chevron" size={14} /> : <ChevronsUpDown className="model-trigger-chevron" size={14} />}
    </button>
    {open && <div className="model-flyout-container">
      <div className="model-menu-card main-menu">
        {providers.map((provider) => <button type="button" key={provider.id} className={`model-category-row ${hoveredId === provider.id ? "hovered" : ""} ${provider.id === currentProviderId ? "has-active" : ""}`} onMouseEnter={(event) => showProvider(provider.id, event.currentTarget)} onClick={(event) => showProvider(provider.id, event.currentTarget)}>
          <ChevronLeft className="category-arrow" size={16} />
          <span className="category-name">{provider.name}</span>
          {provider.id === currentProviderId && <Check className="model-card-check" size={15} />}
        </button>)}
      </div>
      {hovered && <div ref={flyoutRef} className="model-menu-card submenu-flyout" style={{ top: flyoutTop }}>
        {hovered.models.length === 0 ? <div className="model-card-empty">No models configured</div> : groupModels(hovered.models).map(([group, models]) => <section className="model-group-section" key={group}>
          <div className="model-menu-header">{group}</div>
          {models.map((model) => {
            const selected = hovered.id === currentProviderId && model.model === currentModel;
            return <button type="button" key={model.id} className={`model-card-item ${selected ? "selected" : ""}`} disabled={busy} onClick={() => void onChange(hovered.id, model.model)}>
              <span className="model-card-name">{model.model}</span>
              {selected && <Check className="model-card-check" size={16} />}
            </button>;
          })}
        </section>)}
      </div>}
    </div>}
  </div>;
}
