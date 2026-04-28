import type { Page } from "playwright-core";

import type { AskSnapshot } from "../snapshotState";
import { noteProviderPressure, noteProviderPressureFromText } from "../timing";

const PROVIDER_GENERATION_FAILURE_CODE =
  "provider_generation_failed_nonretryable";
const PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS = 1_500;

class AskProviderFailure extends Error {
  readonly code = PROVIDER_GENERATION_FAILURE_CODE;
  readonly currentUrl: string;
  readonly retryVisible: boolean;

  constructor(message: string, currentUrl: string, retryVisible: boolean) {
    super(message);
    this.name = "AskProviderFailure";
    this.currentUrl = currentUrl;
    this.retryVisible = retryVisible;
  }
}

export function throwIfProviderGenerationFailed(snapshot: AskSnapshot): void {
  if (snapshot.failureCode !== PROVIDER_GENERATION_FAILURE_CODE) {
    return;
  }
  noteProviderPressure("generation_failed");
  if (snapshot.retryVisible) {
    noteProviderPressure("retry_visible");
  }
  noteProviderPressureFromText(snapshot.failureMessage);
  throw new AskProviderFailure(
    snapshot.failureMessage ??
      "Something went wrong while generating the response. If this issue persists please contact us through our help center.",
    snapshot.currentUrl,
    snapshot.retryVisible,
  );
}

export async function ensureProviderReachable(page: Page): Promise<void> {
  const result = await page.evaluate(async (timeoutMs) => {
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);
    try {
      const response = await fetch(new URL("/", window.location.origin), {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        signal: controller.signal,
      });
      if (!response.ok) {
        return {
          ok: false as const,
          timedOut: false,
          reason: `status ${response.status}`,
        };
      }
      return { ok: true as const };
    } catch (error) {
      return {
        ok: false as const,
        timedOut: controller.signal.aborted,
        reason: error instanceof Error ? error.message : String(error),
      };
    } finally {
      window.clearTimeout(timeoutId);
    }
  }, PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS);
  if (result.ok) {
    return;
  }
  const suffix = result.timedOut ? "timed out" : "failed";
  const reason = result.reason ? `: ${result.reason}` : "";
  throw new Error(
    `provider/network-unavailable: chatgpt preflight request ${suffix}${reason}`,
  );
}
