/** @vitest-environment jsdom */

import React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DataInput } from "@/components/DataInput";
import { localExport } from "../support/localExport";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function setup() {
  const onDataLoaded = vi.fn();
  const view = render(<DataInput onDataLoaded={onDataLoaded} />);
  const textarea = screen.getByRole("textbox", { name: "Export JSON" });
  return { onDataLoaded, textarea, unmount: view.unmount };
}

const invalidExportCases: Array<readonly [string, (data: ReturnType<typeof localExport>) => unknown]> = [
  ["null meta", (data) => ({ ...data, meta: null })],
  ["null summary", (data) => ({ ...data, summary: null })],
  ["missing summary clients", (data) => {
    const summary: Partial<typeof data.summary> = { ...data.summary };
    delete summary.clients;
    return { ...data, summary };
  }],
  ["an out-of-range contribution timestamp", (data) => ({
    ...data,
    contributions: data.contributions.map((day, index) =>
      index === 0 ? { ...day, timestampMs: Number.MAX_VALUE } : day
    ),
  })],
  ["a fractional intensity", (data) => ({
    ...data,
    contributions: data.contributions.map((day, index) => (index === 0 ? { ...day, intensity: 1.5 } : day)),
  })],
  ["a prototype property as a client id", (data) => ({
    ...data,
    summary: { ...data.summary, clients: ["__proto__"] },
    contributions: data.contributions.map((day, index) =>
      index === 0
        ? { ...day, clients: day.clients.map((line, lineIndex) => lineIndex === 0 ? { ...line, client: "__proto__" } : line) }
        : day
    ),
  })],
];

function legacySourceExport() {
  const data = localExport();
  const { clients, ...summary } = data.summary;
  return {
    ...data,
    summary: { ...summary, sources: clients },
    contributions: data.contributions.map(({ clients: dayClients, ...day }) => ({
      ...day,
      sources: dayClients.map(({ client, ...line }) => ({ ...line, source: client })),
    })),
  };
}

describe("local viewer data input", () => {
  it("parses pasted `tokscale graph` JSON", () => {
    const { onDataLoaded, textarea } = setup();
    const parse = screen.getByRole("button", { name: "Parse JSON" });
    expect(parse).toHaveProperty("disabled", true);

    const data = localExport();
    fireEvent.change(textarea, { target: { value: JSON.stringify(data) } });
    expect(parse).toHaveProperty("disabled", false);
    fireEvent.click(parse);
    expect(onDataLoaded).toHaveBeenCalledWith(data);
  });

  it("parses with Ctrl+Enter or Cmd+Enter", () => {
    const { onDataLoaded, textarea } = setup();
    fireEvent.change(textarea, { target: { value: JSON.stringify(localExport()) } });
    fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true });
    fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });
    expect(onDataLoaded).toHaveBeenCalledTimes(2);
  });

  it("loads graph exports from before the source-to-client rename", () => {
    const { onDataLoaded, textarea } = setup();
    const legacy = legacySourceExport();
    fireEvent.change(textarea, { target: { value: JSON.stringify(legacy) } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));

    expect(onDataLoaded).toHaveBeenCalledTimes(1);
    const loaded = onDataLoaded.mock.calls[0][0];
    expect(loaded.summary.clients).toEqual(legacy.summary.sources);
    expect(loaded.contributions[0].clients[0].client).toBe(legacy.contributions[0].sources[0].source);
  });

  it("reports broken JSON and the wrong shape as alerts on the field", () => {
    const { onDataLoaded, textarea } = setup();
    fireEvent.change(textarea, { target: { value: "{not json" } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("alert").textContent).toMatch(/^Invalid JSON: /);
    expect(textarea.getAttribute("aria-invalid")).toBe("true");
    expect(textarea.getAttribute("aria-describedby")).toContain(screen.getByRole("alert").id);

    fireEvent.change(textarea, { target: { value: JSON.stringify({ meta: {} }) } });
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("alert").textContent).toContain("Invalid data format");
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it.each(invalidExportCases)("rejects pasted exports with %s", (_label, makeInvalid) => {
    const { onDataLoaded, textarea } = setup();
    fireEvent.change(textarea, { target: { value: JSON.stringify(makeInvalid(localExport())) } });
    fireEvent.click(screen.getByRole("button", { name: "Parse JSON" }));
    expect(screen.getByRole("alert").textContent).toContain("Invalid data format");
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it.each(invalidExportCases)("rejects chosen exports with %s", async (_label, makeInvalid) => {
    const { onDataLoaded } = setup();
    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    const file = new File([JSON.stringify(makeInvalid(localExport()))], "usage.json", { type: "application/json" });
    fireEvent.change(input as HTMLInputElement, { target: { files: [file] } });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Invalid data format"));
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("loads the sample data", async () => {
    const data = localExport();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: true, status: 200, json: async () => data })),
    );
    const { onDataLoaded } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Load sample data" }));
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
    expect(fetch).toHaveBeenCalledWith("/sample-data.json");
  });

  it("loads a chosen .json file in the browser", async () => {
    const data = localExport();
    const { onDataLoaded } = setup();
    expect(screen.getByRole("button", { name: "Choose file" })).toBeTruthy();
    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    expect(input?.accept).toBe(".json,application/json");
    const file = new File([JSON.stringify(data)], "usage.json", { type: "application/json" });
    fireEvent.change(input as HTMLInputElement, { target: { files: [file] } });
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
  });

  it.each(["chosen", "dropped"] as const)("rejects an oversized %s export before reading it", async (source) => {
    const fileReader = vi.fn();
    vi.stubGlobal("FileReader", fileReader);
    const { onDataLoaded } = setup();
    const file = new File(["{}"], "huge.json", { type: "application/json" });
    Object.defineProperty(file, "size", { value: 64 * 1024 * 1024 + 1 });

    if (source === "chosen") {
      const input = document.querySelector<HTMLInputElement>('input[type="file"]') as HTMLInputElement;
      fireEvent.change(input, { target: { files: [file] } });
    } else {
      const card = screen.getByRole("region", { name: "Load token usage data" });
      fireEvent.drop(card, { dataTransfer: { types: ["Files"], files: [file] } });
    }

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("under 64 MiB"));
    expect(fileReader).not.toHaveBeenCalled();
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("keeps the later file when two reads finish out of order", async () => {
    const readers: ControlledFileReader[] = [];

    class ControlledFileReader {
      result: string | null = null;
      error: Error | null = null;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;

      constructor() {
        readers.push(this);
      }

      readAsText() {}

      resolve(text: string) {
        this.result = text;
        this.onload?.();
      }
    }

    vi.stubGlobal("FileReader", ControlledFileReader);
    const { onDataLoaded } = setup();
    const input = document.querySelector<HTMLInputElement>('input[type="file"]') as HTMLInputElement;
    const first = localExport();
    const second = { ...localExport(), meta: { ...localExport().meta, version: "later-choice" } };

    fireEvent.change(input, { target: { files: [new File(["first"], "first.json", { type: "application/json" })] } });
    fireEvent.change(input, { target: { files: [new File(["second"], "second.json", { type: "application/json" })] } });
    expect(readers).toHaveLength(2);

    await act(async () => readers[1].resolve(JSON.stringify(second)));
    expect(onDataLoaded).toHaveBeenCalledTimes(1);
    expect(onDataLoaded).toHaveBeenLastCalledWith(second);

    await act(async () => readers[0].resolve(JSON.stringify(first)));
    expect(onDataLoaded).toHaveBeenCalledTimes(1);
  });

  it("ignores a file read that finishes after unmount", async () => {
    let finishRead: ((text: string) => void) | undefined;

    class ControlledFileReader {
      result: string | null = null;
      error: Error | null = null;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;

      readAsText() {
        finishRead = (text: string) => {
          this.result = text;
          this.onload?.();
        };
      }
    }

    vi.stubGlobal("FileReader", ControlledFileReader);
    const { onDataLoaded, unmount } = setup();
    const input = document.querySelector<HTMLInputElement>('input[type="file"]') as HTMLInputElement;
    fireEvent.change(input, { target: { files: [new File(["pending"], "pending.json", { type: "application/json" })] } });
    expect(finishRead).toBeTypeOf("function");

    unmount();
    await act(async () => finishRead?.(JSON.stringify(localExport())));
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("loads a dropped export and shows where to drop it while dragging", async () => {
    const data = localExport();
    const { onDataLoaded } = setup();
    const card = screen.getByRole("region", { name: "Load token usage data" });
    const file = new File([JSON.stringify(data)], "usage.json", { type: "application/json" });
    const dataTransfer = { types: ["Files"], files: [file], dropEffect: "none" };

    fireEvent.dragEnter(card, { dataTransfer });
    expect(card.textContent).toContain("Drop the export to load it");
    fireEvent.drop(card, { dataTransfer });
    expect(card.textContent).not.toContain("Drop the export to load it");
    await waitFor(() => expect(onDataLoaded).toHaveBeenCalledWith(data));
  });

  it("refuses a dropped file that is not JSON", async () => {
    const { onDataLoaded } = setup();
    const card = screen.getByRole("region", { name: "Load token usage data" });
    const file = new File(["not an export"], "notes.txt", { type: "text/plain" });
    fireEvent.drop(card, { dataTransfer: { types: ["Files"], files: [file] } });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("notes.txt is not a .json file"));
    expect(onDataLoaded).not.toHaveBeenCalled();
  });

  it("explains how to get an export", () => {
    setup();
    const steps = screen.getByRole("region", { name: "How to get your data" });
    expect(steps.textContent).toContain("bunx tokscale");
    expect(steps.textContent).toContain("tokscale graph");
    expect(steps.textContent).toContain("tokscale graph --output usage.json");
  });
});
