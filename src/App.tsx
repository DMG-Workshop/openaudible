import { audioDir, homeDir, join } from "@tauri-apps/api/path";
import { useEffect, useState } from "react";
import { JobQueue } from "./components/JobQueue";
import { LibraryView } from "./components/LibraryView";
import { MetadataSidebar } from "./components/MetadataSidebar";
import { Toolbar } from "./components/Toolbar";
import { api, type BookDetail, type ScanReport } from "./lib/api";
import { useJobEvents, useJobs } from "./state/jobs";

export default function App() {
  useJobEvents();
  const upsert = useJobs((s) => s.upsert);
  const [selected, setSelected] = useState<number | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [outDir, setOutDir] = useState<string | null>(null);
  const [convertError, setConvertError] = useState<string | null>(null);

  useEffect(() => {
    // Some Linux setups have no XDG music directory; fall back to the home folder.
    (async () => {
      const base = await audioDir().catch(() => homeDir());
      setOutDir(await join(base, "AudioShelf"));
    })().catch((e) => setConvertError(`Could not resolve output folder: ${String(e)}`));
  }, []);

  const convert = async (book: BookDetail) => {
    if (!outDir) return;
    try {
      const ids = await api.convert([book.id], { outDir, bitrateKbps: 64, overwrite: false });
      ids.forEach((id) =>
        upsert({ id, bookId: book.id, title: book.title, status: "queued", progress: 0, error: null }),
      );
      setConvertError(null);
    } catch (e) {
      setConvertError(String(e));
    }
  };

  return (
    <div className="app">
      <Toolbar
        onScanned={(_: ScanReport) => setRefreshKey((k) => k + 1)}
      />
      {convertError && <p className="notice job__error" role="alert">{convertError}</p>}
      <div className="shell">
        <main>
          <LibraryView selectedId={selected} onSelect={setSelected} refreshKey={refreshKey} />
        </main>
        <MetadataSidebar bookId={selected} onConvert={convert} />
        <JobQueue />
      </div>
    </div>
  );
}
