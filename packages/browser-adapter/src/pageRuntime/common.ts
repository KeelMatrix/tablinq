import type { Locator, Page } from "playwright-core";

import { safePageUrl } from "../capabilityProbe";
import { interactionJitterMs, type InteractionJitterKind } from "../timing";
import { resolveConversationUrl } from "../urls";

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
  await sleep(Math.min(interactionJitterMs(kind), boundedMaxMs));
}

export async function clickWithInteractionJitter(
  locator: Locator,
  kind: InteractionJitterKind,
  options?: Parameters<Locator["click"]>[0],
): Promise<void> {
  await pauseForInteraction(kind);
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
