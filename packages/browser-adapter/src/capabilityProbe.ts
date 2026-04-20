import type { Page } from "playwright-core";

import type {
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
} from "./contracts";

export function telemetryKey(capability: string, strategyId: string): string {
  return `${capability}:${strategyId}`;
}

export async function captureDomSnapshot(
  page: Page,
  neighborhoods: DomNeighborhoodSnapshot[],
): Promise<string> {
  return page.evaluate((entries) => {
    const excerpt = document.body?.innerHTML?.slice(0, 16_000) ?? "";
    return JSON.stringify(
      {
        title: document.title,
        url: window.location.href,
        neighborhoods: entries,
        body_excerpt: excerpt,
      },
      null,
      2,
    );
  }, neighborhoods);
}

export async function captureAriaSnapshot(page: Page): Promise<string> {
  return page.evaluate(() => {
    const isVisible = (element: Element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const computed = window.getComputedStyle(element);
      return (
        !element.hidden &&
        element.getAttribute("aria-hidden") !== "true" &&
        computed.display !== "none" &&
        computed.visibility !== "hidden"
      );
    };
    const accessibleName = (element: Element) =>
      (element.getAttribute("aria-label") ?? element.textContent ?? "")
        .replace(/\s+/g, " ")
        .trim();
    const nodes = Array.from(
      document.querySelectorAll(
        [
          "[role]",
          "button",
          "input",
          "textarea",
          "a",
          "[contenteditable='true']",
        ].join(", "),
      ),
    )
      .filter((element) => isVisible(element))
      .slice(0, 120)
      .map((element) => ({
        tag: element.tagName.toLowerCase(),
        role: element.getAttribute("role"),
        name: accessibleName(element),
      }));
    return JSON.stringify(nodes, null, 2);
  });
}

export function inferAuthenticated(currentUrl: string): boolean {
  try {
    const url = new URL(currentUrl);
    const localHost =
      url.hostname === "127.0.0.1" || url.hostname === "localhost";
    if (localHost) {
      const path = url.pathname.toLowerCase();
      if (path.includes("login") || path.includes("auth")) {
        return false;
      }
    }
    return (
      url.hostname === "chatgpt.com" ||
      url.hostname === "chat.openai.com" ||
      localHost
    );
  } catch {
    return false;
  }
}

export async function safeTitle(page: Page): Promise<string | null> {
  try {
    return await page.title();
  } catch {
    return null;
  }
}

export function safePageUrl(page: Page): string | null {
  try {
    return page.url();
  } catch {
    return null;
  }
}

export function logRecoveryDecision(
  kind: "ask" | "research",
  details: Record<string, unknown>,
): void {
  try {
    process.stderr.write(
      `[tablinq-recovery] ${kind} ${JSON.stringify(details)}\n`,
    );
  } catch {
    // Recovery logging is opportunistic and must not affect browser control.
  }
}

export function hasUsableCapabilityReport(
  report: CapabilityReportPayload,
): boolean {
  return report.matrix.composer_ready || report.matrix.submit_available;
}
