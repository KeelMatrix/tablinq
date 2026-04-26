import { writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import type { Page } from "playwright-core";
import { captureAriaSnapshot, captureDomSnapshot } from "../capabilityProbe";
import type {
  BrowserDiagnosticsSnapshot,
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
} from "../contracts";
import { detectBrowserAvailability } from "../launch";
import type { CapabilityStrategyCache } from "./capabilityCache";

export interface ProfileAccessSnapshot {
  profileAccessible: boolean;
  profileAccessError: string | null;
}

export class DiagnosticsBundleCapture {
  constructor(private readonly profileDir: string) {}

  async checkProfileAccess(): Promise<ProfileAccessSnapshot> {
    try {
      await mkdir(this.profileDir, { recursive: true });
      writeFileSync(
        resolve(this.profileDir, ".tablinq-profile-check"),
        "ok",
        "utf8",
      );
      return { profileAccessible: true, profileAccessError: null };
    } catch (error) {
      return {
        profileAccessible: false,
        profileAccessError:
          error instanceof Error ? error.message : String(error),
      };
    }
  }

  async captureBundleScreenshot(
    page: Page,
    bundleDir: string,
  ): Promise<string> {
    const screenshotPath = resolve(bundleDir, "failure.png");
    await mkdir(dirname(screenshotPath), { recursive: true });
    await page.screenshot({ fullPage: true, path: screenshotPath });
    return screenshotPath;
  }
}

const defaultCapabilityMatrix = {
  composer_ready: false,
  submit_available: false,
  model_picker_visible: false,
  thinking_effort_visible: false,
  search_mode_visible: false,
  research_mode_visible: false,
  upload_control_visible: false,
  downloadable_artifacts_observable: false,
};

export interface BrowserDiagnosticsCaptureInput {
  capabilityCache: CapabilityStrategyCache;
  diagnosticsCapture: DiagnosticsBundleCapture;
  page: Page | undefined;
  bundleDir?: string;
  currentBrowserPath?: string | null;
  lastCapabilityReport: CapabilityReportPayload | null;
  lastUiSignature: string | null;
  lastDomNeighborhoods: DomNeighborhoodSnapshot[];
  onScreenshotCaptured(path: string): void;
}

export async function captureBrowserDiagnostics({
  capabilityCache,
  diagnosticsCapture,
  page,
  bundleDir,
  currentBrowserPath,
  lastCapabilityReport,
  lastUiSignature,
  lastDomNeighborhoods,
  onScreenshotCaptured,
}: BrowserDiagnosticsCaptureInput): Promise<BrowserDiagnosticsSnapshot> {
  capabilityCache.hydrate();
  const selectorPack = capabilityCache.ensureSelectorPackLoaded();
  const { profileAccessible, profileAccessError } =
    await diagnosticsCapture.checkProfileAccess();

  const screenshotPath =
    bundleDir && page
      ? await diagnosticsCapture
          .captureBundleScreenshot(page, bundleDir)
          .then((path) => {
            onScreenshotCaptured(path);
            return path;
          })
          .catch(() => null)
      : null;

  const browserAvailability = detectBrowserAvailability(
    process.platform,
    currentBrowserPath ?? undefined,
  );

  return {
    current_url: page?.url() ?? null,
    capability_matrix: lastCapabilityReport?.matrix ?? defaultCapabilityMatrix,
    strategy_attempts: capabilityCache.buildStrategyAttempts({
      lastCapabilityReport,
      lastUiSignature,
      lastDomNeighborhoods,
    }),
    dom_snapshot: page
      ? await captureDomSnapshot(page, lastDomNeighborhoods)
      : "",
    aria_snapshot: page ? await captureAriaSnapshot(page) : "",
    screenshot_path: screenshotPath,
    selector_pack: selectorPack
      ? {
          pack_id: selectorPack.pack_id,
          version: `v${selectorPack.format_version}`,
          updated_at: selectorPack.updated_at ?? null,
          path: capabilityCache.selectorPackPath,
        }
      : null,
    strategy_telemetry: capabilityCache.getStrategyTelemetrySnapshots(),
    browser_detected: browserAvailability.detected,
    detected_browser_path: browserAvailability.path,
    profile_accessible: profileAccessible,
    profile_access_error: profileAccessError,
  };
}
