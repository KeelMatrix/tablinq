import path from "node:path";
import { expect } from "vitest";
import {
  BrowserAdapter,
  type AskResultSnapshot,
  type BrowserAdapterOptions,
  type BrowserOpenCommand,
  type ResearchProgressSnapshot,
  type BrowserStatusSnapshot,
  type StreamChunkSnapshot,
} from "../../src/browserAdapter";
import {
  type HarnessUrlOptions,
  type createBrowserIntegrationHarness,
  withHarnessUrl,
} from "../browser-adapter.integration.helpers";

type BrowserIntegrationHarness = ReturnType<
  typeof createBrowserIntegrationHarness
>;

type ManagedAdapterOptions = {
  workspacePrefix?: string;
  adapterOptions?: Partial<
    Omit<
      BrowserAdapterOptions,
      "profileDir" | "downloadsDir" | "defaultTargetUrl"
    >
  >;
};

export async function createTrackedManagedAdapter(
  harness: BrowserIntegrationHarness,
  targetUrl: string,
  options: ManagedAdapterOptions = {},
) {
  const root = await harness.createWorkspace(
    options.workspacePrefix ?? "browser",
  );
  const adapter = harness.trackAdapter(
    new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: targetUrl,
      ...options.adapterOptions,
    }),
  );
  return { adapter, root };
}

export async function openTrackedManagedAdapter(
  harness: BrowserIntegrationHarness,
  targetUrl: string,
  options: ManagedAdapterOptions & {
    openCommand?: Partial<BrowserOpenCommand>;
  } = {},
) {
  const { adapter, root } = await createTrackedManagedAdapter(
    harness,
    targetUrl,
    options,
  );
  const snapshot = await adapter.open({
    targetUrl,
    headless: true,
    ...options.openCommand,
  });
  return { adapter, root, snapshot };
}

export async function openManagedHarnessAdapter(
  harness: BrowserIntegrationHarness,
  baseUrl: string,
  urlOptions: HarnessUrlOptions = {},
  options: ManagedAdapterOptions & {
    openCommand?: Partial<BrowserOpenCommand>;
  } = {},
) {
  const targetUrl = withHarnessUrl(baseUrl, urlOptions);
  const opened = await openTrackedManagedAdapter(harness, targetUrl, options);
  return { ...opened, targetUrl };
}

export function captureDeltaChunks() {
  const chunks: string[] = [];
  return {
    chunks,
    onChunk(chunk: StreamChunkSnapshot) {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text);
      }
    },
    text() {
      return chunks.join("");
    },
  };
}

export function captureResearchStatuses() {
  const statuses: string[] = [];
  return {
    statuses,
    onUpdate(update: ResearchProgressSnapshot) {
      statuses.push(update.status);
    },
  };
}

export function expectResearchHandoffStatuses(statuses: string[]) {
  expect(statuses).toContain("starting");
  expect(statuses).toContain("waiting_for_sources");
  expect(statuses).toContain("running");
}

export function expectCompletedAskResult(
  result: AskResultSnapshot,
  outputText: string,
) {
  expect(result.output_text).toBe(outputText);
  expect(result.conversation_id).toMatch(/^conv-/);
  expect(result.message_id).toMatch(/^msg-/);
}

export function expectManagedHarnessOpenSnapshot(
  snapshot: BrowserStatusSnapshot,
  targetUrl: string,
) {
  expect(snapshot.running).toBe(true);
  expect(snapshot.current_url).toContain(targetUrl);
  expect(snapshot.authenticated).toBe(true);
  expect(snapshot.extension_connected).toBe(true);
  expect(snapshot.capability_report?.matrix.composer_ready).toBe(true);
  expect(snapshot.attach_mode).toBe("managed_only");
  expect(snapshot.browser_owner).toBe("managed");
  expect(snapshot.cdp_attached).toBe(false);
  expect(snapshot.target_tab_found).toBe(true);
  expect(snapshot.target_tab_id).toBeTruthy();
  expect(snapshot.target_workspace_key).toBe(new URL(targetUrl).toString());
  expect(snapshot.debug_endpoint).toBe("http://127.0.0.1:9222");
  expect(snapshot.auth_source).toBe("page");
  expect(snapshot.extension_required).toBe(true);
  expect(snapshot.last_capability_report_at).toBeTruthy();
}

export function expectHarnessCapabilityMatrix(snapshot: BrowserStatusSnapshot) {
  expect(snapshot.capability_report?.matrix).toEqual({
    composer_ready: true,
    submit_available: true,
    model_picker_visible: true,
    thinking_effort_visible: true,
    search_mode_visible: true,
    research_mode_visible: true,
    upload_control_visible: true,
    downloadable_artifacts_observable: true,
  });
  expect(
    snapshot.capability_report?.aliases.model_classes.instant,
  ).toBeTruthy();
  expect(
    snapshot.capability_report?.aliases.model_classes.thinking,
  ).toBeTruthy();
  expect(snapshot.capability_report?.aliases.efforts.standard).toBeTruthy();
  expect(snapshot.capability_report?.aliases.efforts.extended).toBeTruthy();
}
