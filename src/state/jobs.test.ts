import { beforeEach, describe, expect, it } from "vitest";
import { useJobs } from "./jobs";

describe("job store", () => {
  beforeEach(() => useJobs.setState({ jobs: {} }));

  it("merges partial updates into an existing job", () => {
    const { upsert } = useJobs.getState();
    upsert({ id: 1, bookId: 9, title: "Dune", status: "queued", progress: 0, error: null });
    upsert({ id: 1, status: "running", progress: 0.5 });
    expect(useJobs.getState().jobs[1]).toMatchObject({ title: "Dune", status: "running", progress: 0.5 });
  });

  it("never moves a finished job back to running from a late event", () => {
    const { upsert } = useJobs.getState();
    upsert({ id: 2, bookId: 9, title: "Dune", status: "done", progress: 1, error: null });
    upsert({ id: 2, status: "running", progress: 0.4 });
    expect(useJobs.getState().jobs[2].status).toBe("done");
  });
});
