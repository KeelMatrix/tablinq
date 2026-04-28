import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  adaptivePollIntervalMs,
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
import { markdownCompatibleRecoveredText } from "../src/snapshotState";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import { openManagedHarnessAdapter } from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";
import { maybeCloseManagedBrowserOnRunningIdentity } from "./helpers/recoveryFlow";

const harness = installBrowserAdapterHarness();

describe("browser adapter recovery: primitives", () => {
  it("backs off adaptive polling during longer recovery waits", () => {
    const immediate = adaptivePollIntervalMs("recovery", 100, 1_000);
    const settled = adaptivePollIntervalMs("recovery", 2_000, 1_000);
    const prolonged = adaptivePollIntervalMs("recovery", 6_000, 1_000);

    expect(immediate).toBeLessThan(settled);
    expect(settled).toBeLessThan(prolonged);
  });

  it("treats inserted markdown list lead-ins as compatible during ask recovery", () => {
    const emitted = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Large, well-known local bakery chain with a wide",
    ].join("\n");
    const final = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Lien Hoa Bakery",
      "  Large, well-known local bakery chain with a wide selection of breads, cakes, and Vietnamese pastries.",
    ].join("\n");

    expect(markdownCompatibleRecoveredText(emitted, final)).toBe(true);
  });

  it("does not treat unrelated inserted markdown content as compatible recovery text", () => {
    const emitted = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Large, well-known local bakery chain with a wide",
    ].join("\n");
    const final = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Unexpected Bakery",
      "  Known only for coffee, not a wide bakery selection.",
    ].join("\n");

    expect(markdownCompatibleRecoveredText(emitted, final)).toBe(false);
  });
});
