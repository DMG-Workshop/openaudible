import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type BookSummary = {
  id: number;
  title: string;
  author: string;
  narrator: string;
  durationMs: number | null;
  coverPath: string | null;
  missing: boolean;
};

export type ChapterRow = { idx: number; startMs: number; endMs: number; title: string };

export type BookDetail = {
  id: number;
  title: string;
  author: string;
  narrator: string;
  series: string | null;
  seriesIndex: number | null;
  description: string | null;
  codec: string | null;
  bitrateBps: number | null;
  durationMs: number | null;
  chapters: ChapterRow[];
};

export type ScanReport = { filesSeen: number; filesProbed: number; books: number };

export type ConvertOpts = { outDir: string; bitrateKbps: number; overwrite: boolean };

export type JobEvent = {
  id: number;
  status: "queued" | "running" | "done" | "failed" | "cancelled";
  progress: number; // 0..1
  error: string | null;
};

export const api = {
  search: (query: string, offset = 0, limit = 120) =>
    invoke<BookSummary[]>("search_books", { query, offset, limit }),
  getBook: (id: number) => invoke<BookDetail | null>("get_book", { id }),
  scan: (roots: string[]) => invoke<ScanReport>("scan_library", { roots }),
  convert: (bookIds: number[], opts: ConvertOpts) =>
    invoke<number[]>("enqueue_convert", { bookIds, opts }),
  cancel: (jobId: number) => invoke<void>("cancel_job", { jobId }),
};

/** Subscribes to backend job events. Returns a promise of the unlisten function. */
export const onJobProgress = (cb: (e: JobEvent) => void) =>
  listen<JobEvent>("job://progress", (e) => cb(e.payload));

/** Human-readable duration, e.g. "1h 05m" or "4:07". */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}:${String(s).padStart(2, "0")}`;
}
