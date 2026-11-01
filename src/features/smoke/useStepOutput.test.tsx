import { act, renderHook, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { describe, expect, it, vi } from "vitest";
import type { OutputChunk } from "@/lib/ipc";
import { useStepOutput, type OutputSource } from "./useStepOutput";

/** A stand-in for the core: a file that grows as the test appends to it. */
function fakeFile() {
  let file = "";
  let grew: ((runId: string, stepId: string) => void) | null = null;
  const bytes = (text: string) => new TextEncoder().encode(text).length;
  const source: OutputSource = {
    read: vi.fn(async (_run, _step, _stream, from): Promise<OutputChunk> => {
      const encoded = new TextEncoder().encode(file);
      const piece = new TextDecoder().decode(encoded.slice(from));
      return { text: piece, from, next: encoded.length, size: encoded.length };
    }),
    subscribe: vi.fn(async (handler) => {
      grew = handler;
      return () => {
        grew = null;
      };
    }),
  };
  return {
    source,
    append(text: string) {
      file += text;
      grew?.("r1", "ping");
    },
    /** Written, with no notice: the runner tells the window every half second at most. */
    appendQuietly(text: string) {
      file += text;
    },
    size: () => bytes(file),
  };
}

describe("useStepOutput", () => {
  it("reads the file once, then reads on from where it was each time it grows", async () => {
    const file = fakeFile();
    file.append("first line\n");
    const { result } = renderHook(() => useStepOutput("r1", "ping", "stdout", true, file.source));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.text).toBe("first line\n");

    await act(async () => file.append("second line\n"));
    await waitFor(() => expect(result.current.text).toBe("first line\nsecond line\n"));
    // Every read after the first started where the one before ended.
    const froms = vi.mocked(file.source.read).mock.calls.map((call) => call[3]);
    expect(froms).toEqual([0, 11]);
    expect(result.current.skipped).toBe(false);
  });

  it("ignores other steps' growth and reads once more when the step ends", async () => {
    const file = fakeFile();
    file.append("a\n");
    const { result, rerender } = renderHook(
      ({ live }) => useStepOutput("r1", "ping", "stdout", live, file.source),
      { initialProps: { live: true } },
    );
    await waitFor(() => expect(result.current.text).toBe("a\n"));
    const reads = () => vi.mocked(file.source.read).mock.calls.length;
    const before = reads();
    await act(async () => {
      const grew = vi.mocked(file.source.subscribe).mock.calls[0]![0];
      grew("r1", "other-step");
      grew("r2", "ping");
    });
    expect(reads()).toBe(before);

    // The step ended after it wrote its last line, with no notice for that line.
    file.appendQuietly("b\n");
    vi.mocked(file.source.read).mockClear();
    rerender({ live: false });
    await waitFor(() => expect(result.current.text).toBe("a\nb\n"));
    expect(vi.mocked(file.source.read).mock.calls.map((call) => call[3])).toEqual([2]);
  });

  it("reads an ended step's file once even when React mounts the view twice", async () => {
    // Development builds mount every effect twice. The first mount's read must not stand in
    // for the second's, or a step that never grows again shows nothing.
    const file = fakeFile();
    file.appendQuietly("all done\n");
    const { result } = renderHook(() => useStepOutput("r1", "ping", "stdout", false, file.source), {
      wrapper: StrictMode,
    });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.text).toBe("all done\n");
  });
});
