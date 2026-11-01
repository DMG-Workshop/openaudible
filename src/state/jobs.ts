import { useEffect } from "react";
import { create } from "zustand";
import { onJobProgress, type JobEvent } from "../lib/api";

export type Job = JobEvent & { bookId: number; title: string };

type JobsState = {
  jobs: Record<number, Job>;
  upsert: (patch: Partial<Job> & { id: number }) => void;
};

export const useJobs = create<JobsState>((set) => ({
  jobs: {},
  upsert: (patch) =>
    set((s) => {
      const previous = s.jobs[patch.id];
      // A late progress event must never move a finished job back to running.
      if (previous && (previous.status === "done" || previous.status === "cancelled" || previous.status === "failed") && patch.status === "running") {
        return s;
      }
      return { jobs: { ...s.jobs, [patch.id]: { ...previous, ...patch } as Job } };
    }),
}));

/** Mount once at the app root. Subscribes to backend job events for the app's lifetime. */
export function useJobEvents() {
  const upsert = useJobs((s) => s.upsert);
  useEffect(() => {
    const unlisten = onJobProgress(upsert);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [upsert]);
}
