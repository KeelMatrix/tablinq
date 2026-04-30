import type { Page } from "playwright-core";

import type { AskExecutionCommand } from "../contracts";
import { safePageUrl } from "../capabilityProbe";
import {
  applyAskConfiguration,
  clickSubmit,
  fillComposer,
  ensureConfiguredSearchMode,
  readAskBaseline,
  sleep,
  startNewConversation,
  uploadAttachments,
  waitForInteractionSurface,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import type { AskBaseline } from "../snapshotState";
import { SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS } from "../timing";
import { conversationIdFromUrl } from "../urls";

import { ensureProviderReachable } from "./askProviderFailure";
import type { AskTiming } from "./askTiming";
import type { BrowserExecutionHost } from "./types";

export async function openAskExecutionPage(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
): Promise<Page> {
  const requiresFreshNavigation =
    command.newConversation && !command.conversationId;
  const page = await host.openExecutionPage(
    host.resolveAskTarget(command.conversationId, command.newConversation),
    requiresFreshNavigation,
  );
  await ensureProviderReachable(page);
  return page;
}

export async function prepareAskSubmission(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  timing: AskTiming,
): Promise<{ page: Page; askBaseline: AskBaseline }> {
  if (
    (command.newConversation && !command.conversationId) ||
    (!command.conversationId && !conversationIdFromUrl(safePageUrl(page)))
  ) {
    await startNewConversation(page);
  }
  await waitForInteractionSurface(page, { reloadOnUnavailable: true });
  const askBaseline = await readAskBaseline(page);
  await host.refreshCapabilities(page);
  if (
    command.modelClass ||
    command.effort ||
    (command.mode && command.mode !== "normal")
  ) {
    const alreadyConfigured = await visibleComposerConfigMatchesRequest(
      page,
      command,
    );
    if (!alreadyConfigured) {
      page = await applyAskConfiguration(
        page,
        command,
        host.getLastCapabilityReport(),
      );
      await host.refreshCapabilities(page);
    }
  }
  await uploadAttachments(
    page,
    command.files ?? [],
    host.getLastCapabilityReport(),
  );
  await fillComposer(page, command.prompt);
  if ((command.files ?? []).length > 0) {
    await waitForSubmitReadyAfterUploads(
      page,
      Math.min(
        SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
        Math.max(1, timing.timeoutAt - Date.now()),
      ),
    );
  }
  if (command.mode === "search") {
    await sleep(1_000);
    page = await host.openExecutionPage(safePageUrl(page) ?? page.url());
    await ensureConfiguredSearchMode(page);
  }
  await maybeApplySubmitJitter(command.submitJitterMs, timing.timeoutAt);
  await clickSubmit(page);
  return { page, askBaseline };
}

async function visibleComposerConfigMatchesRequest(
  page: Page,
  command: AskExecutionCommand,
): Promise<boolean> {
  if (command.mode && command.mode !== "normal") {
    return false;
  }
  if (!command.modelClass && !command.effort) {
    return false;
  }
  return page
    .evaluate(
      ({
        modelClass,
        effort,
      }: {
        modelClass?: "instant" | "thinking" | null;
        effort?: "none" | "standard" | "extended" | null;
      }) => {
        const texts = Array.from(
          document.querySelectorAll(
            "button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
          ),
        )
          .filter((element) => {
            if (!(element instanceof HTMLElement)) {
              return false;
            }
            const style = window.getComputedStyle(element);
            return (
              style.display !== "none" &&
              style.visibility !== "hidden" &&
              style.opacity !== "0" &&
              element.getClientRects().length > 0
            );
          })
          .map((element) =>
            (element.textContent ?? "")
              .replace(/\s+/g, " ")
              .trim()
              .toLowerCase(),
          );
        if (texts.length === 0) {
          return false;
        }
        const modelMatches =
          !modelClass ||
          texts.some((text) =>
            modelClass === "instant"
              ? text.includes("instant") || text.includes("5.3")
              : text.includes("thinking") ||
                text.includes("standard") ||
                text.includes("extended") ||
                text.includes("5.4") ||
                text.includes("5.5"),
          );
        const effortMatches =
          !effort ||
          effort === "none" ||
          texts.some((text) => text.includes(effort));
        return modelMatches && effortMatches;
      },
      {
        modelClass: command.modelClass,
        effort: command.effort,
      },
    )
    .catch(() => false);
}

async function maybeApplySubmitJitter(
  submitJitterMs: number | undefined,
  timeoutAt: number,
): Promise<void> {
  const jitterMs = Math.max(0, Math.trunc(submitJitterMs ?? 0));
  if (jitterMs === 0) {
    return;
  }
  const remainingMs = Math.max(0, timeoutAt - Date.now() - 1);
  if (remainingMs === 0) {
    return;
  }
  await sleep(Math.min(jitterMs, remainingMs));
}
