import type { Page } from "playwright-core";

import { safePageUrl } from "../capabilityProbe";
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
