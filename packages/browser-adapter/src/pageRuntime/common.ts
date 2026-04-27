import type { Locator, Page } from "playwright-core";

import { safePageUrl } from "../capabilityProbe";
import {
  adaptivePollIntervalMs,
  highRiskActionCooldownMs,
  highRiskActionForInteractionKind,
  highRiskPacingEnabled,
  interactionJitterMs,
  quietSurfaceStableWindowMs,
  quietSurfaceTimeoutMs,
  type HighRiskActionKind,
  type InteractionJitterKind,
} from "../timing";
import { resolveConversationUrl } from "../urls";

interface QuietSurfaceSnapshot {
  surfacePresent: boolean;
  surfaceBusy: boolean;
  attachmentsProcessing: boolean;
  navigating: boolean;
  quietForMs: number;
}

export async function resetConversationIdentity(page: Page): Promise<void> {
  const currentUrl =
    safePageUrl(page) ??
    (await page.evaluate(() => {
      return window.location.href;
    }));
  const resetUrl = resolveConversationUrl(currentUrl, null);
  await page.evaluate((nextUrl: string) => {
    delete document.body.dataset.conversationId;
    delete document.body.dataset.lastMessageId;
    history.replaceState({}, "", nextUrl);
  }, resetUrl);
}

export async function scrollToBottomIfNeeded(page: Page): Promise<void> {
  await page
    .evaluate(() => {
      const bottomTolerancePx = 8;
      const candidates = [
        document.scrollingElement,
        document.documentElement,
        document.body,
        ...Array.from(
          document.querySelectorAll(
            [
              "main",
              "[role='main']",
              "[data-testid*='conversation']",
              "[class*='thread']",
              "[class*='scroll']",
              "[class*='overflow-y-auto']",
              "[class*='overflow-auto']",
            ].join(", "),
          ),
        ),
      ];

      const seen = new Set<Element>();
      let best: {
        element: Element;
        scrollHeight: number;
        clientHeight: number;
        scrollTop: number;
        scrollablePx: number;
      } | null = null;

      for (const element of candidates) {
        if (!element || seen.has(element)) {
          continue;
        }
        seen.add(element);
        const scrollHeight = element.scrollHeight;
        const clientHeight = element.clientHeight;
        const scrollablePx = scrollHeight - clientHeight;
        if (scrollablePx <= bottomTolerancePx) {
          continue;
        }
        const entry = {
          element,
          scrollHeight,
          clientHeight,
          scrollTop: element.scrollTop,
          scrollablePx,
        };
        if (!best || entry.scrollablePx > best.scrollablePx) {
          best = entry;
        }
      }

      if (!best) {
        return false;
      }

      const remainingPx =
        best.scrollHeight - best.clientHeight - best.scrollTop;
      if (remainingPx <= bottomTolerancePx) {
        return false;
      }

      if (
        best.element === document.scrollingElement ||
        best.element === document.documentElement ||
        best.element === document.body
      ) {
        window.scrollTo({ top: best.scrollHeight, behavior: "auto" });
      }
      best.element.scrollTop = best.scrollHeight;
      return true;
    })
    .catch(() => undefined);
}

export async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export async function pauseForInteraction(
  kind: InteractionJitterKind,
  maxMs = Number.POSITIVE_INFINITY,
): Promise<void> {
  const boundedMaxMs = Math.max(0, maxMs);
  if (boundedMaxMs === 0) {
    return;
  }
  const delayMs = Math.min(interactionJitterMs(kind), boundedMaxMs);
  if (delayMs <= 0) {
    return;
  }
  await sleep(delayMs);
}

export async function pauseForHighRiskAction(
  action: HighRiskActionKind,
  maxMs = Number.POSITIVE_INFINITY,
): Promise<void> {
  const boundedMaxMs = Math.max(0, maxMs);
  if (boundedMaxMs === 0) {
    return;
  }
  const delayMs = Math.min(highRiskActionCooldownMs(action), boundedMaxMs);
  if (delayMs <= 0) {
    return;
  }
  await sleep(delayMs);
}

export async function waitForQuietInteractionSurface(
  page: Page,
  action: HighRiskActionKind,
  target?: Locator,
  timeoutMs = quietSurfaceTimeoutMs(action),
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = startedAt + Math.max(0, timeoutMs);
  const stableWindowMs = quietSurfaceStableWindowMs(action);

  await page
    .waitForLoadState("domcontentloaded", {
      timeout: Math.min(Math.max(1, timeoutMs), 1_000),
    })
    .catch(() => undefined);

  while (Date.now() < deadline) {
    const snapshot = await readQuietSurfaceSnapshot(page).catch(() => null);
    const actionable = await isLocatorActionable(target);
    if (
      snapshot &&
      snapshot.surfacePresent &&
      !snapshot.surfaceBusy &&
      !snapshot.attachmentsProcessing &&
      !snapshot.navigating &&
      snapshot.quietForMs >= stableWindowMs &&
      actionable
    ) {
      return true;
    }

    await sleep(
      Math.min(
        adaptivePollIntervalMs("stabilize", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  return false;
}

export async function prepareHighRiskInteraction(
  page: Page,
  action: HighRiskActionKind,
  target?: Locator,
  maxMs = Number.POSITIVE_INFINITY,
): Promise<void> {
  if (!highRiskPacingEnabled()) {
    return;
  }

  const startedAt = Date.now();
  const boundedMaxMs = Math.max(0, maxMs);
  const gateTimeoutMs = Number.isFinite(boundedMaxMs)
    ? Math.min(quietSurfaceTimeoutMs(action), boundedMaxMs)
    : quietSurfaceTimeoutMs(action);

  if (gateTimeoutMs > 0) {
    await waitForQuietInteractionSurface(page, action, target, gateTimeoutMs);
  }

  const remainingPauseBudgetMs = Number.isFinite(boundedMaxMs)
    ? Math.max(0, boundedMaxMs - (Date.now() - startedAt))
    : Number.POSITIVE_INFINITY;
  await pauseForHighRiskAction(action, remainingPauseBudgetMs);
}

export async function clickWithInteractionJitter(
  locator: Locator,
  kind: InteractionJitterKind,
  options?: Parameters<Locator["click"]>[0],
  page?: Page,
): Promise<void> {
  const action = highRiskActionForInteractionKind(kind);
  if (page && action) {
    await prepareHighRiskInteraction(page, action, locator);
  } else {
    await pauseForInteraction(kind);
  }
  await locator.click(options);
}

export async function pressShortcutWithInteractionJitter(
  page: Page,
  shortcut: string,
  maxMs = Number.POSITIVE_INFINITY,
): Promise<void> {
  await pauseForInteraction("keyboard_submit", maxMs);
  await page.keyboard.press(shortcut).catch(() => undefined);
}

export function isTransientPageError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("Execution context was destroyed") ||
    message.includes("Cannot find context") ||
    message.includes("Target page, context or browser has been closed") ||
    message.includes("Frame has been detached") ||
    message.includes("navigation")
  );
}

async function isLocatorActionable(
  locator: Locator | undefined,
): Promise<boolean> {
  if (!locator) {
    return true;
  }

  const visible = await locator.isVisible().catch(() => false);
  if (!visible) {
    return false;
  }

  return locator
    .evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const ariaDisabled = element.getAttribute("aria-disabled") === "true";
      const dataDisabled = element.getAttribute("data-disabled") === "true";
      const busy =
        element.getAttribute("aria-busy") === "true" ||
        element.dataset.busy === "true" ||
        element.dataset.loading === "true";
      const nativeDisabled =
        "disabled" in element &&
        Boolean((element as HTMLButtonElement | HTMLInputElement).disabled);
      return !ariaDisabled && !dataDisabled && !busy && !nativeDisabled;
    })
    .catch(() => false);
}

async function readQuietSurfaceSnapshot(
  page: Page,
): Promise<QuietSurfaceSnapshot> {
  return page.evaluate(() => {
    const globalState = window as Window & {
      __tablinqQuietSurfaceState__?: {
        root: HTMLElement | null;
        observer: MutationObserver | null;
        lastMutationAt: number;
        lastNavigationAt: number;
        lastUrl: string;
      };
    };

    const now =
      typeof performance !== "undefined" &&
      typeof performance.now === "function"
        ? performance.now()
        : Date.now();

    const isVisible = (element: Element | null): element is HTMLElement => {
      if (!(element instanceof HTMLElement) || element.hidden) {
        return false;
      }
      const style = window.getComputedStyle(element);
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        style.opacity !== "0"
      );
    };

    const firstVisible = (selectors: string[], root: ParentNode = document) => {
      for (const selector of selectors) {
        for (const element of Array.from(root.querySelectorAll(selector))) {
          if (isVisible(element)) {
            return element;
          }
        }
      }
      return null;
    };

    const composer = firstVisible([
      "#prompt-textarea",
      "[data-slot='composer']",
      "[data-testid='composer-root'] textarea",
      "[data-testid='composer-root'] [contenteditable='true']",
      "textarea[aria-label*='Message']",
      "[role='textbox'][aria-label*='Message']",
      "textarea",
      "[role='textbox']",
      "[contenteditable='true']",
    ]);
    const composerRoot =
      composer?.closest("[data-testid='composer-root']") ??
      composer?.closest("form") ??
      composer?.closest("section") ??
      composer?.closest("main") ??
      document.body;
    const popupRoot =
      firstVisible([
        "[role='menu']",
        "[role='listbox']",
        "[data-radix-popper-content-wrapper]",
        "[data-headlessui-portal]",
        "[data-floating-ui-portal]",
      ]) ?? null;
    const observedRoot =
      popupRoot?.parentElement ??
      (composerRoot instanceof HTMLElement ? composerRoot : document.body);

    const currentUrl = window.location.href;
    const previous = globalState.__tablinqQuietSurfaceState__;
    if (
      !previous ||
      previous.root !== observedRoot ||
      !observedRoot.isConnected
    ) {
      previous?.observer?.disconnect();
      const state = {
        root: observedRoot,
        observer: null as MutationObserver | null,
        lastMutationAt: now,
        lastNavigationAt: now,
        lastUrl: currentUrl,
      };
      const observer = new MutationObserver(() => {
        state.lastMutationAt =
          typeof performance !== "undefined" &&
          typeof performance.now === "function"
            ? performance.now()
            : Date.now();
      });
      observer.observe(observedRoot, {
        subtree: true,
        childList: true,
        attributes: true,
        characterData: true,
      });
      state.observer = observer;
      globalState.__tablinqQuietSurfaceState__ = state;
    }

    const state = globalState.__tablinqQuietSurfaceState__!;
    if (state.lastUrl !== currentUrl) {
      state.lastUrl = currentUrl;
      state.lastNavigationAt = now;
    }

    const submit =
      firstVisible(
        [
          "[data-testid='composer-root'] [data-testid='send-button']",
          "[data-testid='send-button']",
          "button[aria-label='Send prompt']",
          "button[aria-label='Submit prompt']",
          "input[type='submit']",
        ],
        popupRoot ?? observedRoot,
      ) ??
      firstVisible([
        "[data-testid='composer-root'] [data-testid='send-button']",
        "[data-testid='send-button']",
        "button[aria-label='Send prompt']",
        "button[aria-label='Submit prompt']",
        "input[type='submit']",
      ]);

    const surfaceBusy =
      submit instanceof HTMLElement &&
      (submit.getAttribute("aria-busy") === "true" ||
        submit.dataset.busy === "true" ||
        submit.dataset.loading === "true");

    const attachmentsProcessing =
      document.body.dataset.uploadProcessing === "true" ||
      document.body.dataset.attachmentsProcessing === "true" ||
      Boolean(
        firstVisible(
          [
            "[data-slot='upload-processing']",
            "[data-upload-processing='true']",
            "[data-upload-state='processing']",
            "[data-upload-state='uploading']",
            "[data-testid*='upload-progress']",
            "[data-testid*='attachment-progress']",
            "[data-testid*='upload-processing']",
            "[role='progressbar']",
          ],
          observedRoot,
        ),
      );

    return {
      surfacePresent: Boolean(composer || submit),
      surfaceBusy: Boolean(surfaceBusy),
      attachmentsProcessing,
      navigating: document.readyState === "loading",
      quietForMs: Math.max(
        0,
        now - Math.max(state.lastMutationAt, state.lastNavigationAt),
      ),
    };
  });
}
