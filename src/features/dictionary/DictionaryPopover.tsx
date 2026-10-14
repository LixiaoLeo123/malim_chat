import { useEffect, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { LoaderCircle, X } from "lucide-react";
import { api } from "../../api";
import type { DictionaryResponse } from "../../types";

const dictionaries = ["russian_en", "german_en", "english_zh"] as const;
type Dictionary = typeof dictionaries[number];

export function DictionaryPopover({ word, anchor, onClose }: { word: string; anchor: { x: number; y: number }; onClose: () => void }) {
  const popoverRef = useRef<HTMLElement>(null);
  const [position, setPosition] = useState({ left: anchor.x, top: anchor.y, maxHeight: window.innerHeight - anchor.y - 12 });
  const [dictionary, setDictionary] = useState<Dictionary>(() => { const stored = localStorage.getItem("malim-dictionary"); return dictionaries.includes(stored as Dictionary) ? stored as Dictionary : (/[Ѐ-ӿ]/.test(word) ? "russian_en" : "english_zh"); }); const [data, setData] = useState<DictionaryResponse | null>(null); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => { localStorage.setItem("malim-dictionary", dictionary); }, [dictionary]);
  useEffect(() => {
    const place = () => {
      const rect = popoverRef.current?.getBoundingClientRect();
      if (!rect) return;
      const margin = 12;
      const left = Math.max(margin, Math.min(anchor.x - 12, window.innerWidth - rect.width - margin));
      setPosition({ left, top: anchor.y + 8, maxHeight: Math.max(56, window.innerHeight - anchor.y - 20) });
    };
    const frame = window.requestAnimationFrame(place); window.addEventListener("resize", place);
    return () => { window.cancelAnimationFrame(frame); window.removeEventListener("resize", place); };
  }, [anchor.x, anchor.y, word, data, busy]);
  useEffect(() => {
    const outside = (event: PointerEvent) => { if (!popoverRef.current?.contains(event.target as Node)) onClose(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("pointerdown", outside, true); document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("keydown", escape); };
  }, [onClose]);
  useEffect(() => { let live = true; setBusy(true); setError(null); setData(null); void api.dictionary(word, dictionary).then((response) => { if (live) setData(response); }).catch((cause) => { if (live) setError(cause instanceof Error ? cause.message : "Dictionary lookup failed."); }).finally(() => { if (live) setBusy(false); }); return () => { live = false; }; }, [word, dictionary]);
  return <aside ref={popoverRef} className="dictionary-popover" role="dialog" aria-label={`Dictionary lookup for ${word}`} style={position}><header><div><strong>{word}</strong><select value={dictionary} onChange={(event) => setDictionary(event.target.value as Dictionary)}><option value="english_zh">English - Chinese</option><option value="german_en">German - English</option><option value="russian_en">Russian - English</option></select></div><button className="icon-button" aria-label="Close dictionary" onClick={onClose}><X size={17} /></button></header>{busy && <p className="dictionary-loading"><LoaderCircle className="spin" size={16} />Looking up...</p>}{error && <p className="form-error">{error}</p>}{data?.entries.map((entry, index) => <DictionaryEntryView key={`${entry.headword}-${index}`} entry={entry} dictionary={dictionary} />)}{data && data.entries.length === 0 && <p className="muted">No local entry found.</p>}</aside>;
}

function DictionaryEntryView({ entry, dictionary }: { entry: DictionaryResponse["entries"][number]; dictionary: DictionaryResponse["dictionary"] }) {
  if (entry.definition_html) return <article className="dictionary-entry malim-entry" dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(entry.definition_html, { ADD_ATTR: ["target"] }) }} />;
  return <article className="dictionary-entry structured"><div className="dictionary-head"><div><h3>{entry.headword}</h3>{entry.pronunciation && <span>/{entry.pronunciation}/</span>}</div>{entry.labels.length > 0 && <small>{entry.labels.join(" · ")}</small>}</div>{entry.forms.length > 0 && <div className="dictionary-forms"><b>Forms</b><div className="dictionary-pills">{entry.forms.slice(0, 12).map((value) => <span key={value}>{value}</span>)}</div></div>}{entry.translations.length > 0 && <section className="dictionary-senses"><h4>Translations</h4><ol>{entry.translations.map((value) => <li key={value}>{value}</li>)}</ol></section>}{entry.definitions.length > 0 && <section className="dictionary-senses"><h4>{dictionary === "german_en" ? "German entry" : "Definition"}</h4><ol>{entry.definitions.slice(0, 12).map((value, sense) => <li key={`${sense}-${value}`}>{value}</li>)}</ol></section>}</article>;
}
