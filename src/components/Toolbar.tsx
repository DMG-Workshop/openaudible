import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api, type ScanReport } from "../lib/api";

export function Toolbar({ onScanned }: { onScanned: (report: ScanReport) => void }) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const addFolders = async () => {
    const picked = await open({ directory: true, multiple: true, title: "Choose audiobook folders" });
    if (!picked) return;
    const roots = Array.isArray(picked) ? picked : [picked];
    setBusy(true);
    setMessage("Scanning…");
    try {
      const report = await api.scan(roots);
      setMessage(`Scanned ${report.filesSeen} files, ${report.filesProbed} new or changed, ${report.books} books.`);
      onScanned(report);
    } catch (e) {
      setMessage(`Scan failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <header className="toolbar">
      <h1>AudioShelf</h1>
      <button type="button" onClick={addFolders} disabled={busy}>
        {busy ? "Scanning…" : "Add folder"}
      </button>
      <span className="muted" role="status">{message}</span>
    </header>
  );
}
