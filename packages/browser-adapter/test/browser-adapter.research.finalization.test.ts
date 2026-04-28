import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { finalizeResearch } from "../src/execution/finalization";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import {
  captureResearchStatuses,
  expectResearchHandoffStatuses,
  openManagedHarnessAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";
import {
  fakeArtifactPage,
  fakeResearchHost,
  fakeResearchSnapshot,
} from "./helpers/researchFinalization";

const harness = installBrowserAdapterHarness();

describe("browser adapter research: finalization", () => {
  it("emits the full final-safe research report during finalization", async () => {
    const updates: string[] = [];
    const state = {
      emittedText: "Research prefix",
      nextSeq: 3,
    };

    const result = await finalizeResearch(
      fakeResearchHost(),
      {
        jobId: "job-research-final-suffix",
        query: "final suffix",
      },
      fakeArtifactPage(),
      state,
      fakeResearchSnapshot("Research prefix suffix"),
      (update) => {
        if (update.chunk_kind === "delta" && update.chunk_text) {
          updates.push(update.chunk_text);
        }
      },
    );

    expect(updates).toEqual(["Research prefix suffix"]);
    expect(state).toEqual({
      emittedText: "Research prefix suffix",
      nextSeq: 4,
    });
    expect(result.report_text).toBe("Research prefix suffix");
  });

  it("does not emit duplicate research finalization suffixes", async () => {
    const updates: string[] = [];
    const state = {
      emittedText: "Research prefix suffix",
      nextSeq: 3,
    };

    const result = await finalizeResearch(
      fakeResearchHost(),
      {
        jobId: "job-research-final-no-duplicate",
        query: "final duplicate",
      },
      fakeArtifactPage(),
      state,
      fakeResearchSnapshot("Research prefix suffix"),
      (update) => {
        if (update.chunk_kind === "delta" && update.chunk_text) {
          updates.push(update.chunk_text);
        }
      },
    );

    expect(updates).toEqual([]);
    expect(state).toEqual({
      emittedText: "Research prefix suffix",
      nextSeq: 3,
    });
    expect(result.report_text).toBe("Research prefix suffix");
  });

  it("emits divergent finalized research as a final-safe full delta", async () => {
    const updates: string[] = [];
    const state = {
      emittedText: "Draft research text",
      nextSeq: 3,
    };

    const result = await finalizeResearch(
      fakeResearchHost(),
      {
        jobId: "job-research-final-divergent",
        query: "final divergent",
      },
      fakeArtifactPage(),
      state,
      fakeResearchSnapshot("Final research report"),
      (update) => {
        if (update.chunk_kind === "delta" && update.chunk_text) {
          updates.push(update.chunk_text);
        }
      },
    );

    expect(updates).toEqual(["Final research report"]);
    expect(state).toEqual({
      emittedText: "Final research report",
      nextSeq: 4,
    });
    expect(result.report_text).toBe("Final research report");
  });
});
