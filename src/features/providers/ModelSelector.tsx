import { useEffect, useRef, useState } from "react";
import { Check, ChevronRight, ChevronsUpDown, Cpu, LoaderCircle } from "lucide-react";
import type { Conversation, Provider, ProviderModel } from "../../types";

interface ModelDesc {
  tag?: string;
  desc?: string;
}

/** Human-friendly subtitle and tag annotations for common model families. */
const MODEL_DESCRIPTIONS: Record<string, ModelDesc> = {
  // OpenAI
  "gpt-4o": { tag: "Omni", desc: "Flagship versatile multimodal model" },
  "gpt-4o-mini": { tag: "Fast", desc: "Lightweight and cost-efficient" },
  "o1": { tag: "Reasoning", desc: "Deep reasoning and complex problem solving" },
  "o1-mini": { tag: "Reasoning", desc: "Fast reasoning for math and coding" },
  "o3-mini": { tag: "Reasoning", desc: "High-intelligence reasoning with configurable effort" },
  "gpt-4-turbo": { desc: "High-capability model for complex tasks" },
  "chatgpt-4o-latest": { desc: "Dynamic ChatGPT-4o research model" },

  // Anthropic Claude
  "claude-3-7-sonnet-latest": { tag: "Hybrid", desc: "Hybrid instant and extended thinking" },
  "claude-3-5-sonnet-latest": { tag: "Flagship", desc: "Leading coding and analysis capability" },
  "claude-3-5-haiku-latest": { tag: "Fast", desc: "Ultra-fast response for everyday tasks" },
  "claude-3-opus-latest": { desc: "Deep analysis and long-form writing" },

  // Google Gemini
  "gemini-2.0-flash": { tag: "Flash", desc: "Next-gen multimodal speed and reasoning" },
  "gemini-2.0-flash-thinking-exp": { tag: "Thinking", desc: "Visual and textual reasoning preview" },
  "gemini-2.0-pro-exp": { desc: "Advanced reasoning and coding performance" },
  "gemini-1.5-pro": { desc: "Large context analysis up to 2M tokens" },
  "gemini-1.5-flash": { desc: "High frequency and cost-sensitive tasks" },

  // DeepSeek
  "deepseek-chat": { tag: "V3", desc: "Versatile conversational and general model" },
  "deepseek-reasoner": { tag: "R1", desc: "Open reasoning model with chain of thought" },

  // xAI Grok
  "grok-2-latest": { desc: "Advanced reasoning and real-world understanding" },
  "grok-beta": { desc: "Early access Grok intelligence" },
};

function lookupDesc(model: string): ModelDesc | undefined {
  if (MODEL_DESCRIPTIONS[model]) return MODEL_DESCRIPTIONS[model];
  const lower = model.toLowerCase();
  for (const [key, desc] of Object.entries(MODEL_DESCRIPTIONS)) {
    if (lower === key.toLowerCase() || lower.includes(key.toLowerCase())) return desc;
  }
  if (lower.includes("thinking") || lower.includes("reasoner") || lower.includes("reasoning")) {
    return { tag: "Thinking", desc: "Extended reasoning capability" };
  }
  if (lower.includes("flash") || lower.includes("mini") || lower.includes("haiku")) {
    return { tag: "Fast", desc: "Quick and cost-effective" };
  }
  if (lower.includes("pro") || lower.includes("opus") || lower.includes("plus")) {
    return { tag: "Pro", desc: "Advanced intelligence" };
  }
  return undefined;
}

interface FlyoutItem {
  providerId: string;
  providerName: string;
  model: ProviderModel;
  displayName: string;
  desc?: ModelDesc;
}

export function ModelSelector({
  conversation,
  providers,
  open,
  busy,
  onToggle,
  onChange,
}: {
  conversation: Conversation;
  providers: Provider[];
  open: boolean;
  busy: boolean;
  onToggle: () => void;
  onChange: (providerId: string, model: string) => Promise<void>;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const activeProvider = providers.find((p) => p.id === conversation.model_provider_id);
  const currentProviderId = conversation.model_provider_id ?? providers[0]?.id ?? "";
  const currentModel = conversation.model ?? activeProvider?.models[0]?.model ?? "";

  // The primary family currently pinned in the top section
  const [pinnedFamily, setPinnedFamily] = useState<string>("");
  // Currently hovered category that opens the flyout submenu (null = no submenu)
  const [activeCategory, setActiveCategory] = useState<string | null>(null);

  // Close when clicking outside
  useEffect(() => {
    if (!open) return;
    function handleClickOutside(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onToggle();
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [open, onToggle]);

  // Reset active flyout whenever menu closes
  useEffect(() => {
    if (!open) setActiveCategory(null);
  }, [open]);

  // Organize all configured models across providers into:
  // 1. Categories (grouped by provider or model.group_name)
  // 2. A default primary family to pin at the top
  const { categories, currentCategoryName } = (() => {
    const cats: {
      name: string;
      items: FlyoutItem[];
      isSingleProvider: boolean;
    }[] = [];

    // Grouping strategy:
    // If a provider has multiple distinct group_names, use those groups or provider name.
    // If provider names are standard (e.g. OpenAI, Anthropic, DeepSeek, Grok), use provider name.
    providers.forEach((prov) => {
      // Check if this provider has sub-groups
      const groupMap = new Map<string, ProviderModel[]>();
      prov.models.forEach((m) => {
        const g = m.group_name?.trim() || prov.name;
        const list = groupMap.get(g) ?? [];
        list.push(m);
        groupMap.set(g, list);
      });

      // If provider only has 1 group or group === "General", group under provider name
      const hasDistinctGroups = groupMap.size > 1 && !groupMap.has("General");

      if (hasDistinctGroups) {
        groupMap.forEach((models, groupName) => {
          cats.push({
            name: `${prov.name} · ${groupName}`,
            isSingleProvider: false,
            items: models.map((m) => ({
              providerId: prov.id,
              providerName: prov.name,
              model: m,
              displayName: m.model,
              desc: lookupDesc(m.model),
            })),
          });
        });
      } else {
        cats.push({
          name: prov.name,
          isSingleProvider: true,
          items: prov.models.map((m) => ({
            providerId: prov.id,
            providerName: prov.name,
            model: m,
            displayName: m.model,
            desc: lookupDesc(m.model),
          })),
        });
      }
    });

    // Find which category the currently active model belongs to
    let foundCatName = cats[0]?.name ?? "";
    for (const cat of cats) {
      if (cat.items.some((it) => it.providerId === currentProviderId && it.model.model === currentModel)) {
        foundCatName = cat.name;
        break;
      }
    }

    return { categories: cats, currentCategoryName: foundCatName };
  })();

  // Keep pinnedFamily in sync with active model's category when opening/changing
  useEffect(() => {
    if (!pinnedFamily && currentCategoryName) {
      setPinnedFamily(currentCategoryName);
    }
  }, [currentCategoryName, pinnedFamily]);

  const effectivePinned = pinnedFamily || currentCategoryName || categories[0]?.name;
  const pinnedCategory = categories.find((c) => c.name === effectivePinned) ?? categories[0];
  const otherCategories = categories.filter((c) => c !== pinnedCategory);

  const activeFlyoutCategory = categories.find((c) => c.name === activeCategory);

  function handleSelect(item: FlyoutItem) {
    void onChange(item.providerId, item.model.model);
  }

  const selectedProvider = providers.find((p) => p.id === currentProviderId);

  return (
    <div className="model-selector" ref={containerRef}>
      <button
        type="button"
        className="model-button"
        aria-expanded={open}
        disabled={busy}
        onClick={onToggle}
      >
        <Cpu className="model-trigger-icon" size={15} />
        <span className="model-trigger-name">{currentModel || "Choose model"}</span>
        {selectedProvider && <span className="model-trigger-badge">{selectedProvider.name}</span>}
        {busy ? (
          <LoaderCircle className="spin model-trigger-chevron" size={14} />
        ) : (
          <ChevronsUpDown className="model-trigger-chevron" size={14} />
        )}
      </button>

      {open && (
        <div className="model-flyout-container">
          {/* Main Primary Menu */}
          <div className="model-menu-card main-menu">
            {/* Pinned Primary Family (Direct Selection) */}
            {pinnedCategory && (
              <div className="model-menu-section">
                <div className="model-menu-header">{pinnedCategory.name}</div>
                <div className="model-card-list">
                  {pinnedCategory.items.map((item) => {
                    const isSelected =
                      item.providerId === currentProviderId && item.model.model === currentModel;
                    return (
                      <button
                        type="button"
                        key={`${item.providerId}-${item.model.id}`}
                        className={`model-card-item ${isSelected ? "selected" : ""}`}
                        disabled={busy}
                        onClick={() => handleSelect(item)}
                      >
                        <div className="model-card-info">
                          <div className="model-card-title">
                            <span className="model-card-name">{item.displayName}</span>
                            {item.desc?.tag && (
                              <span className="model-card-tag">{item.desc.tag}</span>
                            )}
                          </div>
                          {item.desc?.desc && (
                            <div className="model-card-desc">{item.desc.desc}</div>
                          )}
                        </div>
                        {isSelected && <Check className="model-card-check" size={18} />}
                      </button>
                    );
                  })}
                </div>
              </div>
            )}

            {/* Other Categories with flyout chevron */}
            {otherCategories.length > 0 && (
              <div className="model-menu-categories">
                {otherCategories.map((cat) => {
                  const hasSelected = cat.items.some(
                    (it) => it.providerId === currentProviderId && it.model.model === currentModel
                  );
                  const isHovered = activeCategory === cat.name;
                  return (
                    <button
                      type="button"
                      key={cat.name}
                      className={`model-category-row ${isHovered ? "hovered" : ""} ${hasSelected ? "has-active" : ""}`}
                      onMouseEnter={() => setActiveCategory(cat.name)}
                      onClick={() => {
                        // On click/tap, also set as active flyout or pin it
                        setActiveCategory(cat.name);
                        setPinnedFamily(cat.name);
                      }}
                    >
                      <span className="category-name">{cat.name}</span>
                      <ChevronRight className="category-arrow" size={16} />
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Submenu Flyout on hover/select of category */}
          {activeFlyoutCategory && (
            <div
              className="model-menu-card submenu-flyout"
              onMouseEnter={() => setActiveCategory(activeFlyoutCategory.name)}
              onMouseLeave={() => setActiveCategory(null)}
            >
              <div className="model-card-list">
                {activeFlyoutCategory.items.map((item) => {
                  const isSelected =
                    item.providerId === currentProviderId && item.model.model === currentModel;
                  return (
                    <button
                      type="button"
                      key={`${item.providerId}-${item.model.id}`}
                      className={`model-card-item ${isSelected ? "selected" : ""}`}
                      disabled={busy}
                      onClick={() => handleSelect(item)}
                    >
                      <div className="model-card-info">
                        <div className="model-card-title">
                          <span className="model-card-name">{item.displayName}</span>
                          {item.desc?.tag && (
                            <span className="model-card-tag">{item.desc.tag}</span>
                          )}
                        </div>
                        {item.desc?.desc && (
                          <div className="model-card-desc">{item.desc.desc}</div>
                        )}
                      </div>
                      {isSelected && <Check className="model-card-check" size={18} />}
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
