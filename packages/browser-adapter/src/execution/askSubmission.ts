import type { Page } from "playwright-core";

import type { AskExecutionCommand } from "../contracts";
import { safePageUrl } from "../capabilityProbe";
import {
  applyAskConfiguration,
  clickSubmit,
  fillComposer,
  ensureConfiguredSearchMode,
  readAskBaseline,
  resetConversationIdentity,
  sleep,
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
    await resetConversationIdentity(page);
  }
  await waitForInteractionSurface(page, { reloadOnUnavailable: true });
  const askBaseline = await readAskBaseline(page);
  await host.refreshCapabilities(page);
  if (
    command.modelClass ||
    command.effort ||
    (command.mode && command.mode !== "normal")
  ) {
    page = await applyAskConfiguration(
      page,
      command,
      host.getLastCapabilityReport(),
    );
    await host.refreshCapabilities(page);
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
