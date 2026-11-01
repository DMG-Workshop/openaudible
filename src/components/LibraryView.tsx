import { convertFileSrc } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, formatDuration, type BookSummary } from "../lib/api";

const PAGE = 120;

export function LibraryView({
  selectedId,
  onSelect,
  refreshKey,
}: {
  selectedId: number | null;
  onSelect: (id: number) => void;
  /** Bump to reload after a scan. */
  refreshKey: number;
}) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [books, setBooks] = useState<BookSummary[]>([]);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const countRef = useRef(0);
  const seqRef = useRef(0);
  const sentinel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const t = setTimeout(() => setDebounced(query.trim()), 200);
    return () => clearTimeout(t);
  }, [query]);

  // Each new query or refresh restarts paging. A sequence number drops stale responses.
  const fetchPage = useCallback(
    async (reset: boolean) => {
      const seq = ++seqRef.current;
      const offset = reset ? 0 : countRef.current;
      try {
        const page = await api.search(debounced, offset, PAGE);
        if (seq !== seqRef.current) return;
        countRef.current = offset + page.length;
        setBooks((prev) => (reset ? page : [...prev, ...page]));
        setDone(page.length < PAGE);
        setError(null);
      } catch (e) {
        if (seq === seqRef.current) setError(String(e));
      }
    },
    [debounced],
  );

  useEffect(() => {
    void fetchPage(true);
  }, [fetchPage, refreshKey]);

  useEffect(() => {
    const el = sentinel.current;
    if (!el || done) return;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) void fetchPage(false);
      },
      { rootMargin: "400px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [fetchPage, done]);

  return (
    <div className="library">
      <input
        type="search"
        className="search"
        placeholder="Search title, author, narrator or series"
        aria-label="Search library"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {error && <p className="job__error" role="alert">{error}</p>}
      <div className="grid">
        {books.map((b) => (
          <button
            type="button"
            key={b.id}
            className={`card${b.id === selectedId ? " card--selected" : ""}${b.missing ? " card--missing" : ""}`}
            aria-pressed={b.id === selectedId}
            onClick={() => onSelect(b.id)}
          >
            {b.coverPath ? (
              <img src={convertFileSrc(b.coverPath)} alt="" loading="lazy" />
            ) : (
              <div className="cover-fallback" aria-hidden="true">
                {b.title.charAt(0).toUpperCase() || "?"}
              </div>
            )}
            <span className="card__title">{b.title || "Untitled"}</span>
            <span className="card__meta">
              {b.author || "Unknown author"} · {formatDuration(b.durationMs)}
              {b.missing ? " · missing" : ""}
            </span>
          </button>
        ))}
      </div>
      <div ref={sentinel} aria-hidden="true" />
      {books.length === 0 && done && <p className="muted">No matching books. Add a folder to get started.</p>}
    </div>
  );
}
