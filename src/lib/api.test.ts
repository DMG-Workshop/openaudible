import { describe, expect, it } from "vitest";
import { formatDuration } from "./api";

describe("formatDuration", () => {
  it("formats minutes and seconds under an hour", () => {
    expect(formatDuration(4_000)).toBe("0:04");
    expect(formatDuration(187_000)).toBe("3:07");
  });
  it("formats hours with padded minutes", () => {
    expect(formatDuration(3_900_000)).toBe("1h 05m");
  });
  it("handles missing values", () => {
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(Number.NaN)).toBe("—");
  });
});
