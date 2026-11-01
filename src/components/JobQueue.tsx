import { useShallow } from "zustand/react/shallow";
import { api } from "../lib/api";
import { useJobs } from "../state/jobs";

const ACTIVE = new Set(["queued", "running"]);

export function JobQueue() {
  const jobs = useJobs(useShallow((s) => Object.values(s.jobs).sort((a, b) => b.id - a.id)));

  return (
    <section aria-label="Conversion queue" className="queue">
      <h2>Conversions</h2>
      {jobs.length === 0 && <p className="muted">Nothing queued yet.</p>}
      <ul className="job-list">
        {jobs.map((j) => (
          <li key={j.id} className={`job job--${j.status}`}>
            <div className="job__head">
              <span className="job__title">{j.title}</span>
              <span className="job__status">
                {j.status === "running" ? `${Math.round(j.progress * 100)}%` : j.status}
              </span>
              {ACTIVE.has(j.status) && (
                <button
                  type="button"
                  onClick={() => api.cancel(j.id).catch((e) => console.error(e))}
                  aria-label={`Cancel ${j.title}`}
                >
                  Cancel
                </button>
              )}
            </div>
            <progress value={j.progress} max={1} aria-label={`${j.title} progress`} />
            {j.error && <p className="job__error" role="alert">{j.error}</p>}
          </li>
        ))}
      </ul>
    </section>
  );
}
