import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  BrowserAdapter,
  adaptivePollIntervalMs,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  highRiskActionCooldownMs,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  noteProviderPressure,
  noteProviderPressureFromText,
  pacingRandomValueFromSeed,
  providerPressureCooldownMs,
  resetSessionRhythmForTest,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { applyAskSnapshot } from "../src/execution/streaming";
import { loadPageAdapterSource } from "../src/assets";
import {
  scrollToBottomIfNeeded,
  waitForInteractionSurface,
  waitForQuietInteractionSurface,
} from "../src/pageRuntime";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import {
  expectHarnessCapabilityMatrix,
  expectManagedHarnessOpenSnapshot,
  openManagedHarnessAdapter,
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";

const harness = installBrowserAdapterHarness({ resetSessionRhythm: true });

describe("browser adapter foundation: composer input handling", () => {
  it("scrolls the active page to the bottom only when it is not already there", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "managed_only",
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    const page = adapter.getManagedPageForTest()!;
    await page.setViewportSize({ width: 900, height: 600 });
    await page.evaluate(() => {
      document.body.insertAdjacentHTML(
        "beforeend",
        "<div id='scroll-probe' style='height: 3000px'></div>",
      );
      window.scrollTo(0, 0);
    });

    await scrollToBottomIfNeeded(page);
    const firstScrollY = await page.evaluate(() => window.scrollY);
    await scrollToBottomIfNeeded(page);
    const secondScrollY = await page.evaluate(() => window.scrollY);

    expect(firstScrollY).toBeGreaterThan(0);
    expect(secondScrollY).toBe(firstScrollY);

    await adapter.shutdown();
  });

  it("scrolls the composer-owned nested scrollport instead of a larger visible container", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "managed_only",
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    const page = adapter.getManagedPageForTest()!;
    await page.setViewportSize({ width: 900, height: 600 });
    await page.evaluate(() => {
      document.body.innerHTML = `
        <div class="thread-scroll" style="height: 500px; overflow-y: auto">
          <main id="main" style="overflow: visible">
            <div style="height: 2500px">conversation</div>
            <form data-testid="composer-root">
              <textarea id="prompt-textarea" aria-label="Message"></textarea>
            </form>
          </main>
        </div>
      `;
      const scroller = document.querySelector(".thread-scroll");
      if (!(scroller instanceof HTMLElement)) {
        throw new Error("nested scroller missing");
      }
      scroller.scrollTop = 0;
    });

    await scrollToBottomIfNeeded(page);

    const state = await page.evaluate(() => {
      const scroller = document.querySelector(".thread-scroll");
      const main = document.querySelector("#main");
      if (
        !(scroller instanceof HTMLElement) ||
        !(main instanceof HTMLElement)
      ) {
        throw new Error("scroll test nodes missing");
      }
      return {
        scrollerRemaining:
          scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop,
        mainScrollTop: main.scrollTop,
      };
    });

    expect(state.scrollerRemaining).toBeLessThanOrEqual(8);
    expect(state.mainScrollTop).toBe(0);

    await adapter.shutdown();
  });

  it("scales upload acknowledgment timeout nonlinearly by file size and additional files", () => {
    expect(uploadAckTimeoutMsForFiles([])).toBe(10_000);
    expect(uploadAckTimeoutMsForFiles([0])).toBe(10_000);
    expect(uploadAckTimeoutMsForFiles([MAX_UPLOAD_FILE_BYTES])).toBe(40_000);
    expect(uploadAckTimeoutMsForFiles([MAX_UPLOAD_FILE_BYTES / 4])).toBe(
      25_000,
    );
    expect(
      uploadAckTimeoutMsForFiles([
        MAX_UPLOAD_FILE_BYTES,
        MAX_UPLOAD_FILE_BYTES / 4,
      ]),
    ).toBe(62_500);
  });
});
