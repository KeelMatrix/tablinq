import type { Page } from "playwright-core";

import type {
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
} from "./contracts";

export function telemetryKey(capability: string, strategyId: string): string {
  return `${capability}:${strategyId}`;
}

const DOM_SNAPSHOT_MAX_CHARS = 16_000;

async function requestExtensionDomSnapshot(page: Page): Promise<string | null> {
  return page.evaluate((maxChars) => {
    const fallbackSnapshot = () => {
      const root = document.body ?? document.documentElement;
      if (!root || typeof document.createTreeWalker !== "function") {
        return "";
      }
      const lines: string[] = [];
      let total = 0;
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
        acceptNode: (node) => {
          const parent = node.parentElement;
          if (!parent) {
            return NodeFilter.FILTER_REJECT;
          }
          const tag = parent.tagName.toLowerCase();
          if (
            tag === "script" ||
            tag === "style" ||
            tag === "noscript" ||
            tag === "template"
          ) {
            return NodeFilter.FILTER_REJECT;
          }
          const text = node.textContent?.replace(/\s+/g, " ").trim() ?? "";
          return text ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
        },
      });
      while (walker.nextNode()) {
        const text = walker.currentNode.textContent
          ?.replace(/\s+/g, " ")
          .trim();
        if (!text) {
          continue;
        }
        const remaining = maxChars - total;
        if (remaining <= 0) {
          break;
        }
        const excerpt = text.slice(0, remaining);
        lines.push(excerpt);
        total += excerpt.length + 1;
      }
      return lines.join("\n").slice(0, maxChars);
    };

    return new Promise<string>((resolve) => {
      const requestId = `tablinq-dom-${Date.now()}-${Math.random().toString(16).slice(2)}`;
      const cleanup = () => {
        window.removeEventListener("message", onMessage);
        clearTimeout(timeout);
      };
      const onMessage = (event: MessageEvent<unknown>) => {
        const data = event.data as
          | {
              source?: string;
              type?: string;
              request_id?: string;
              dom_snapshot?: string;
            }
          | undefined;
        if (
          event.source !== window ||
          data?.source !== "tablinq-content-script" ||
          data.type !== "GET_DOM_SNAPSHOT_RESULT" ||
          data.request_id !== requestId
        ) {
          return;
        }
        cleanup();
        resolve(
          typeof data.dom_snapshot === "string"
            ? data.dom_snapshot.slice(0, maxChars)
            : fallbackSnapshot(),
        );
      };
      const timeout = window.setTimeout(() => {
        cleanup();
        resolve(fallbackSnapshot());
      }, 250);

      window.addEventListener("message", onMessage);
      window.postMessage(
        {
          source: "tablinq-page",
          type: "GET_DOM_SNAPSHOT",
          request_id: requestId,
        },
        "*",
      );
    });
  }, DOM_SNAPSHOT_MAX_CHARS);
}

export async function captureDomSnapshot(
  page: Page,
  neighborhoods: DomNeighborhoodSnapshot[],
): Promise<string> {
  const domText = await requestExtensionDomSnapshot(page).catch(() => null);
  return page
    .evaluate((entries) => {
      return JSON.stringify(
        {
          title: document.title,
          url: window.location.href,
          neighborhoods: entries,
          dom_text: document.body?.innerText?.replace(/\s+/g, " ").trim() ?? "",
        },
        null,
        2,
      );
    }, neighborhoods)
    .then((raw) => {
      const snapshot = JSON.parse(raw) as {
        title: string;
        url: string;
        neighborhoods: DomNeighborhoodSnapshot[];
        dom_text: string;
      };
      snapshot.dom_text = (domText ?? snapshot.dom_text ?? "").slice(
        0,
        DOM_SNAPSHOT_MAX_CHARS,
      );
      return JSON.stringify(snapshot, null, 2);
    });
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
  if (!recoveryLoggingEnabled()) {
    return;
  }
  try {
    process.stderr.write(
      `[tablinq-recovery] ${kind} ${JSON.stringify(details)}\n`,
    );
  } catch {
    // Recovery logging is opportunistic and must not affect browser control.
  }
}

function recoveryLoggingEnabled(): boolean {
  const flag = process.env.TABLINQ_RECOVERY_DEBUG?.trim().toLowerCase();
  return flag === "1" || flag === "true" || flag === "yes";
}

export function hasUsableCapabilityReport(
  report: CapabilityReportPayload,
): boolean {
  return report.matrix.composer_ready || report.matrix.submit_available;
}
