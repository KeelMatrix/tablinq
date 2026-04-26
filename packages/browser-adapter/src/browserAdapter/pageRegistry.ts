import type { BrowserContext, Page } from "playwright-core";
import { safePageUrl } from "../capabilityProbe";
import type {
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
} from "../contracts";
import { ATTACH_PROBE_GRACE_MS } from "../timing";
import { buildTargetPageId, workspaceKeyFromUrl } from "../urls";

export interface TargetPageState {
  page_id: string;
  last_seen_url: string | null;
  workspace_key: string | null;
  extension_connected: boolean;
  capability_report: CapabilityReportPayload | null;
  ui_signature: string | null;
  dom_neighborhoods: DomNeighborhoodSnapshot[];
  last_capability_report_at: string | null;
  probe_ready_until_ms: number;
}

export class BrowserPageRegistry {
  private readonly observedPages = new WeakSet<Page>();
  private readonly observedContexts = new WeakSet<BrowserContext>();
  private readonly pageStates = new Map<string, TargetPageState>();

  activePageState(pageId: string | null): TargetPageState | undefined {
    return pageId ? this.pageStates.get(pageId) : undefined;
  }

  hasObservedPage(page: Page): boolean {
    return this.observedPages.has(page);
  }

  markPageObserved(page: Page): void {
    this.observedPages.add(page);
  }

  hasObservedContext(context: BrowserContext): boolean {
    return this.observedContexts.has(context);
  }

  markContextObserved(context: BrowserContext): void {
    this.observedContexts.add(context);
  }

  ensurePageState(page: Page, cdpAttached: boolean): TargetPageState {
    const pageId = buildTargetPageId(page);
    const existing = this.pageStates.get(pageId);
    if (existing) {
      return existing;
    }

    const pageUrl = safePageUrl(page);
    const state: TargetPageState = {
      page_id: pageId,
      last_seen_url: pageUrl,
      workspace_key: pageUrl ? workspaceKeyFromUrl(pageUrl) : null,
      extension_connected: false,
      capability_report: null,
      ui_signature: null,
      dom_neighborhoods: [],
      last_capability_report_at: null,
      probe_ready_until_ms: cdpAttached
        ? Date.now() + ATTACH_PROBE_GRACE_MS
        : 0,
    };
    this.pageStates.set(pageId, state);
    return state;
  }

  updatePageLocationState(page: Page, cdpAttached: boolean): TargetPageState {
    const state = this.ensurePageState(page, cdpAttached);
    const currentUrl = safePageUrl(page);
    state.last_seen_url = currentUrl;
    state.workspace_key = currentUrl ? workspaceKeyFromUrl(currentUrl) : null;
    if (cdpAttached) {
      state.probe_ready_until_ms = Math.max(
        state.probe_ready_until_ms,
        Date.now() + ATTACH_PROBE_GRACE_MS,
      );
    }
    return state;
  }

  deletePageState(pageId: string): void {
    this.pageStates.delete(pageId);
  }

  clearPageStates(): void {
    this.pageStates.clear();
  }
}
