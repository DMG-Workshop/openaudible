import { useEffect, useState } from "react";
import { api, formatDuration, type BookDetail } from "../lib/api";

export function MetadataSidebar({
  bookId,
  onConvert,
}: {
  bookId: number | null;
  onConvert: (book: BookDetail) => void;
}) {
  const [book, setBook] = useState<BookDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setError(null);
    if (bookId == null) {
      setBook(null);
      return;
    }
    let live = true;
    api
      .getBook(bookId)
      .then((b) => live && setBook(b))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [bookId]);

  if (error) return <aside className="sidebar" role="alert">{error}</aside>;
  if (!book) return <aside className="sidebar sidebar--empty">Select a book to see its details.</aside>;

  return (
    <aside className="sidebar" aria-label="Book details">
      <h2>{book.title}</h2>
      <dl className="facts">
        <dt>Author</dt>
        <dd>{book.author || "—"}</dd>
        <dt>Narrator</dt>
        <dd>{book.narrator || "—"}</dd>
        <dt>Series</dt>
        <dd>{book.series ? `${book.series} #${book.seriesIndex ?? "?"}` : "—"}</dd>
        <dt>Length</dt>
        <dd>{formatDuration(book.durationMs)}</dd>
        <dt>Format</dt>
        <dd>
          {book.codec ?? "—"}
          {book.bitrateBps ? ` · ${Math.round(book.bitrateBps / 1000)} kbps` : ""}
        </dd>
      </dl>
      {/* Plain text only: never render HTML from file tags. */}
      {book.description && <p className="description">{book.description}</p>}
      <h3>Chapters ({book.chapters.length})</h3>
      <ol className="chapters">
        {book.chapters.map((c) => (
          <li key={c.idx}>
            <time>{formatDuration(c.startMs)}</time> {c.title}
          </li>
        ))}
      </ol>
      <button type="button" className="primary" onClick={() => onConvert(book)}>
        Convert to M4B
      </button>
    </aside>
  );
}
