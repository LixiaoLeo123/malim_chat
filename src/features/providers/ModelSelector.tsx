import { useEffect, useRef, useState } from "react";
import { Check, ChevronLeft, ChevronsUpDown, Cpu, LoaderCircle } from "lucide-react";
import type { Conversation, Provider, ProviderModel } from "../../types";

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

  // Active provider and model from conversation
  const activeProvider =
    providers.find((p) => p.id === conversation.model_provider_id) ?? providers[0];
  const currentProviderId = conversation.model_provider_id ?? activeProvider?.id ?? "";
  const currentModel = conversation.model ?? activeProvider?.models[0]?.model ?? "";

  // The provider pinned in the top section of the main menu (defaults to active conversation provider)
  const [pinnedProviderId, setPinnedProviderId] = useState<string>(currentProviderId);
  // Hovered provider ID whose submenu is open on the left
  const [hoveredProviderId, setHoveredProviderId] = useState<string | null>(null);

  // Keep pinned provider in sync with active conversation when conversation changes
  useEffect(() => {
    if (currentProviderId) {
      setPinnedProviderId(currentProviderId);
    }
  }, [currentProviderId]);

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

  // Reset hovered submenu whenever menu closes
  useEffect(() => {
    if (!open) setHoveredProviderId(null);
  }, [open]);

  const pinnedProvider =
    providers.find((p) => p.id === pinnedProviderId) ?? activeProvider ?? providers[0];
  const otherProviders = providers.filter((p) => p.id !== pinnedProvider?.id);
  const submenuProvider = providers.find((p) => p.id === hoveredProviderId);

  const selectedProvider = providers.find((p) => p.id === currentProviderId);

  function handleSelect(providerId: string, modelName: string) {
    void onChange(providerId, modelName);
  }

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
          {/* Submenu Flyout: fixed strictly on the LEFT of the main menu */}
          {submenuProvider && (
            <div
              className="model-menu-card submenu-flyout"
              onMouseEnter={() => setHoveredProviderId(submenuProvider.id)}
              onMouseLeave={() => setHoveredProviderId(null)}
            >
              <div className="model-menu-header">{submenuProvider.name}</div>
              <div className="model-card-list">
                {submenuProvider.models.length === 0 ? (
                  <div className="model-card-empty">No models configured</div>
                ) : (
                  submenuProvider.models.map((m: ProviderModel) => {
                    const isSelected =
                      submenuProvider.id === currentProviderId && m.model === currentModel;
                    return (
                      <button
                        type="button"
                        key={m.id}
                        className={`model-card-item ${isSelected ? "selected" : ""}`}
                        disabled={busy}
                        onClick={() => handleSelect(submenuProvider.id, m.model)}
                      >
                        <span className="model-card-name">{m.model}</span>
                        {isSelected && <Check className="model-card-check" size={16} />}
                      </button>
                    );
                  })
                )}
              </div>
            </div>
          )}

          {/* Main Primary Menu: fixed position directly under trigger */}
          <div className="model-menu-card main-menu">
            {/* Pinned Provider Models */}
            {pinnedProvider && (
              <div className="model-menu-section">
                <div className="model-menu-header">{pinnedProvider.name}</div>
                <div className="model-card-list">
                  {pinnedProvider.models.length === 0 ? (
                    <div className="model-card-empty">No models configured</div>
                  ) : (
                    pinnedProvider.models.map((m: ProviderModel) => {
                      const isSelected =
                        pinnedProvider.id === currentProviderId && m.model === currentModel;
                      return (
                        <button
                          type="button"
                          key={m.id}
                          className={`model-card-item ${isSelected ? "selected" : ""}`}
                          disabled={busy}
                          onClick={() => handleSelect(pinnedProvider.id, m.model)}
                        >
                          <span className="model-card-name">{m.model}</span>
                          {isSelected && <Check className="model-card-check" size={16} />}
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            )}

            {/* Other Providers from Preferences */}
            {otherProviders.length > 0 && (
              <div className="model-menu-categories">
                {otherProviders.map((provider) => {
                  const hasSelected =
                    provider.id === currentProviderId &&
                    provider.models.some((m) => m.model === currentModel);
                  const isHovered = hoveredProviderId === provider.id;
                  return (
                    <button
                      type="button"
                      key={provider.id}
                      className={`model-category-row ${isHovered ? "hovered" : ""} ${hasSelected ? "has-active" : ""}`}
                      onMouseEnter={() => setHoveredProviderId(provider.id)}
                      onClick={() => {
                        setHoveredProviderId(provider.id);
                        setPinnedProviderId(provider.id);
                      }}
                    >
                      <ChevronLeft className="category-arrow" size={16} />
                      <span className="category-name">{provider.name}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
