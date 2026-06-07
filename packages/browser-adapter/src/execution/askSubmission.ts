import type { Page } from "playwright-core";

import type { AskExecutionCommand } from "../contracts";
import { safePageUrl } from "../capabilityProbe";
import {
  applyAskConfiguration,
  assertVisibleComposerSelection,
  clearConfiguredSearchMode,
  clearIrrelevantComposerChips,
  clickSubmit,
  ensureConfiguredThinkingEffort,
  fillComposer,
  ensureConfiguredEffort,
  ensureConfiguredModelClass,
  ensureConfiguredSearchMode,
  readAskBaseline,
  sleep,
  startNewConversation,
  uploadAttachments,
  waitForInteractionSurface,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import { readComposerAttachmentState } from "../pageRuntime/attachments";
import type { AskBaseline } from "../snapshotState";
import { submitReadyAfterUploadTimeoutMs } from "../timing";
import { conversationIdFromUrl } from "../urls";

import { ensureProviderReachable } from "./askProviderFailure";
import type { AskTiming } from "./askTiming";
import { recoverPromptAttachmentUploadIfNeeded } from "./promptAttachmentRecovery";
import type { BrowserExecutionHost } from "./types";
import { uploadUserPromptTextIfNeeded } from "./userPromptUpload";

export async function openAskExecutionPage(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
): Promise<Page> {
  const requiresFreshNavigation =
    command.newConversation && !command.conversationId;
  const page = await host.openExecutionPage(
    host.resolveAskTarget(command.conversationId, command.newConversation),
    requiresFreshNavigation,
    { providerConversationId: command.conversationId ?? null },
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
  const shouldStartFreshAskConversation =
    (command.newConversation && !command.conversationId) ||
    (!command.conversationId && !conversationIdFromUrl(safePageUrl(page)));
  if (shouldStartFreshAskConversation) {
    await startNewConversation(page);
  }
  await waitForInteractionSurface(page, { reloadOnUnavailable: true });
  await host.refreshCapabilities(page);
  if (command.mode !== "search") {
    await clearConfiguredSearchMode(page);
    if (!command.modelClass && !command.effort) {
      const chipClearState = await clearIrrelevantComposerChips(page);
      if (chipClearState === "cleared") {
        await waitForInteractionSurface(page, { reloadOnUnavailable: true });
        await host.refreshCapabilities(page);
      }
    }
  }
  if (
    command.modelClass ||
    command.effort ||
    (command.mode && command.mode !== "normal")
  ) {
    const alreadyConfigured =
      command.modelClass || command.effort
        ? false
        : await visibleComposerConfigMatchesRequest(page, command);
    if (!alreadyConfigured) {
      page = await applyAskConfiguration(
        page,
        command,
        host.getLastCapabilityReport(),
      );
      await waitForInteractionSurface(page, { reloadOnUnavailable: true });
      await host.refreshCapabilities(page);
    }
  }
  const askBaseline = await readAskBaseline(page);
  const explicitFiles = command.files ?? [];
  await uploadAttachments(page, explicitFiles, host.getLastCapabilityReport());
  const attachmentBaseline = await readComposerAttachmentState(page);
  const uploadedUserPrompt = await uploadUserPromptTextIfNeeded({
    page,
    prompt: command.prompt,
    explicitFiles,
    promptUploadThresholdChars: command.promptUploadThresholdChars,
    attachmentBaseline,
    report: host.getLastCapabilityReport(),
  });
  if (uploadedUserPrompt) {
    await fillComposer(page, uploadedUserPrompt.instruction);
  } else {
    await fillComposer(page, command.prompt);
    await recoverPromptAttachmentUploadIfNeeded(
      page,
      attachmentBaseline,
      timing.timeoutAt,
    );
  }
  if (explicitFiles.length > 0 || uploadedUserPrompt) {
    await waitForSubmitReadyAfterUploads(
      page,
      Math.min(
        submitReadyAfterUploadTimeoutMs(),
        Math.max(1, timing.timeoutAt - Date.now()),
      ),
    );
  }
  if (command.mode === "search") {
    await sleep(1_000);
    page = await host.openExecutionPage(safePageUrl(page) ?? page.url());
    const searchState = await ensureConfiguredSearchMode(page);
    if (searchState === "activated" && (command.modelClass || command.effort)) {
      const report = host.getLastCapabilityReport();
      if (!report) {
        throw new Error(
          "capability missing: no capability report is available for model selection after search activation",
        );
      }
      if (
        command.modelClass === "thinking" &&
        command.effort &&
        command.effort !== "none" &&
        (await ensureConfiguredThinkingEffort(page, report, command.effort))
      ) {
        // Thinking and effort were selected together from the model picker.
      } else if (command.modelClass) {
        await ensureConfiguredModelClass(page, report, command.modelClass);
        if (command.effort) {
          await ensureConfiguredEffort(page, report, command.effort);
        }
      } else if (command.effort) {
        await ensureConfiguredEffort(page, report, command.effort);
      }
    }
    if (uploadedUserPrompt) {
      await fillComposer(page, uploadedUserPrompt.instruction);
    } else {
      await recoverPromptAttachmentUploadIfNeeded(
        page,
        attachmentBaseline,
        timing.timeoutAt,
      );
    }
  }
  await maybeApplySubmitJitter(command.submitJitterMs, timing.timeoutAt);
  await assertVisibleComposerSelection(page, {
    modelClass: command.modelClass ?? null,
    effort: command.effort ?? null,
  });
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
  return false;
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
