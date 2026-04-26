import type { Page } from "playwright-core";

import { loadProbeSource } from "../assets";
import { hasUsableCapabilityReport } from "../capabilityProbe";
import type {
  CapabilityBridgeMessage,
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
  ProbeDocumentOptions,
} from "../contracts";
import { PROBE_TIMEOUT_MS } from "../timing";
import { isTransientPageError, sleep } from "../pageRuntime";
import { CapabilityStrategyCache } from "./capabilityCache";
import type { BrowserPageRegistry, TargetPageState } from "./pageRegistry";

export interface ActiveCapabilitySnapshot {
  extensionConnected: boolean;
  lastCapabilityReport: CapabilityReportPayload | null;
  lastUiSignature: string | null;
  lastDomNeighborhoods: DomNeighborhoodSnapshot[];
}

export class CapabilityProbeController {
  private extensionConnected = false;
  private lastCapabilityReport: CapabilityReportPayload | null = null;
  private lastUiSignature: string | null = null;
  private lastDomNeighborhoods: DomNeighborhoodSnapshot[] = [];

  constructor(
    private readonly pageRegistry: BrowserPageRegistry,
    private readonly capabilityCache: CapabilityStrategyCache,
    private readonly getActivePageId: () => string | null,
    private readonly isCdpAttached: () => boolean,
  ) {}

  get cache(): CapabilityStrategyCache {
    return this.capabilityCache;
  }

  snapshot(): ActiveCapabilitySnapshot {
    return {
      extensionConnected: this.extensionConnected,
      lastCapabilityReport: this.lastCapabilityReport,
      lastUiSignature: this.lastUiSignature,
      lastDomNeighborhoods: [...this.lastDomNeighborhoods],
    };
  }

  activeReportedAt(): string | null {
    return (
      this.pageRegistry.activePageState(this.getActivePageId())
        ?.last_capability_report_at ?? null
    );
  }

  clearActiveState(): void {
    this.extensionConnected = false;
    this.lastCapabilityReport = null;
    this.lastDomNeighborhoods = [];
    this.lastUiSignature = null;
  }

  applyPageStateToActiveTarget(state: TargetPageState | undefined): void {
    if (!state) {
      this.clearActiveState();
      return;
    }
    this.extensionConnected = state.extension_connected;
    this.lastCapabilityReport = state.capability_report;
    this.lastUiSignature = state.ui_signature;
    this.lastDomNeighborhoods = [...state.dom_neighborhoods];
  }

  async refresh(page: Page): Promise<TargetPageState | undefined> {
    if (page.isClosed()) {
      return undefined;
    }

    const cdpAttached = this.isCdpAttached();
    const pageState = this.pageRegistry.updatePageLocationState(
      page,
      cdpAttached,
    );
    const preferredStrategies = this.capabilityCache.preferredStrategiesFor(
      pageState,
      page.url(),
    );
    const selectorPack = this.capabilityCache.ensureSelectorPackLoaded();
    const probeDeadline = cdpAttached
      ? Math.max(pageState.probe_ready_until_ms, Date.now())
      : Date.now();

    for (let attempt = 0; attempt < 10; attempt += 1) {
      try {
        await page.waitForLoadState("domcontentloaded", {
          timeout: PROBE_TIMEOUT_MS,
        });
        const message = await page.evaluate(
          ({
            source,
            options,
          }: {
            source: string;
            options: ProbeDocumentOptions;
          }): CapabilityBridgeMessage => {
            const probeDocument = new Function(
              `${source}\nreturn probeDocument;`,
            )() as (options?: ProbeDocumentOptions) => CapabilityBridgeMessage;
            return probeDocument(options);
          },
          {
            source: loadProbeSource(),
            options: {
              selectorPackId: selectorPack?.pack_id ?? "chatgpt-default",
              selectorPackVersion: selectorPack
                ? `v${selectorPack.format_version}`
                : "v1",
              preferredStrategies,
              selectorPack: selectorPack ?? undefined,
            } satisfies ProbeDocumentOptions,
          },
        );
        if (!hasUsableCapabilityReport(message.report)) {
          if (cdpAttached && Date.now() < probeDeadline) {
            await sleep(150);
            continue;
          }
          if (cdpAttached) {
            return this.recordCapabilityFailure(page);
          }
        }
        return this.recordCapabilitySuccess(page, message);
      } catch (error) {
        if (attempt < 2 && isTransientPageError(error)) {
          await sleep(150 * (attempt + 1));
          continue;
        }
        if (cdpAttached && Date.now() < probeDeadline) {
          await sleep(150);
          continue;
        }
        return this.recordCapabilityFailure(page);
      }
    }

    return this.recordCapabilityFailure(page);
  }

  private recordCapabilitySuccess(
    page: Page,
    message: CapabilityBridgeMessage,
  ): TargetPageState {
    const state = this.pageRegistry.updatePageLocationState(
      page,
      this.isCdpAttached(),
    );
    state.extension_connected = true;
    state.capability_report = message.report;
    state.ui_signature = message.runtime.ui_signature;
    state.dom_neighborhoods = message.runtime.dom_neighborhoods;
    state.last_capability_report_at = message.report.reported_at;
    this.capabilityCache.recordCapabilitySuccess(
      page.url(),
      message,
      this.lastUiSignature,
    );
    if (this.getActivePageId() === state.page_id) {
      this.applyPageStateToActiveTarget(state);
    }
    return state;
  }

  private recordCapabilityFailure(page: Page): TargetPageState {
    const state = this.pageRegistry.updatePageLocationState(
      page,
      this.isCdpAttached(),
    );
    state.extension_connected = false;
    state.capability_report = null;
    state.ui_signature = null;
    state.dom_neighborhoods = [];
    state.last_capability_report_at = null;
    if (this.getActivePageId() === state.page_id) {
      this.applyPageStateToActiveTarget(state);
    }
    return state;
  }
}
