import { Search } from "lucide-react";
import type { Message } from "../../types";
import { sourceHost } from "../../lib/utils";

export function SourceList({ sources }: { sources: Message["search_sources"] }) {
  if (sources.length === 0) return null;
  const queries = [...new Set(sources.map((source) => source.query).filter((query): query is string => Boolean(query)))];
  return <details className="sources"><summary><Search size={14} />{sources.length} web source{sources.length === 1 ? "" : "s"}</summary>{queries.length > 0 && <p className="source-query">Search: {queries.join(" · ")}</p>}<div className="source-list">{sources.map((source, index) => <a href={source.url} target="_blank" rel="noreferrer" key={`${source.url}-${index}`}><span className="source-title">{source.title || sourceHost(source.url)}</span><span className="source-meta">{sourceHost(source.url)} · {source.engine || "SearXNG"}</span>{source.content && <span className="source-snippet">{source.content}</span>}</a>)}</div></details>;
}
