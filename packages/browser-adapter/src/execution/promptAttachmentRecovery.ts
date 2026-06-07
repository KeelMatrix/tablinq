import type { Page } from "playwright-core";

import {
  type ComposerAttachmentState,
  readComposerAttachmentState,
} from "../pageRuntime/attachments";
import { callPageAdapter } from "../pageRuntime/adapter";
import { sleep } from "../pageRuntime/common";
import {
  fillComposer,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime/composer";
import type { SubmitStateSnapshot } from "../snapshotState";
import {
  adaptivePollIntervalMs,
  submitReadyAfterUploadTimeoutMs,
} from "../timing";

export const PROMPT_ATTACHMENT_RECOVERY_PROMPT =
  "Prompt was uploaded as a txt attachment. Treat it as authoritative and follow it exactly.";

const PROMPT_ATTACHMENT_RECOVERY_OBSERVE_MIN_MS = 600;
const PROMPT_ATTACHMENT_RECOVERY_OBSERVE_MAX_MS = 2_000;

type PromptAttachmentRecoveryDecision = {
  shouldRecover: boolean;
  filename: string | null;
  recoveryPrompt: string | null;
};

export function buildPromptAttachmentRecoveryPrompt(
  filename: string | null,
): string {
  if (!filename) {
    return PROMPT_ATTACHMENT_RECOVERY_PROMPT;
  }
  return `Prompt was uploaded as a txt attachment (${filename}). Treat it as authoritative and follow it exactly.`;
}

export function evaluatePromptAttachmentRecovery({
  baseline,
  current,
  submitState,
}: {
  baseline: ComposerAttachmentState;
  current: ComposerAttachmentState;
  submitState: Pick<SubmitStateSnapshot, "composerText">;
}): PromptAttachmentRecoveryDecision {
  if (submitState.composerText.trim()) {
    return noPromptAttachmentRecovery();
  }
  if (newComposerAttachmentCount(baseline, current) <= 0) {
    return noPromptAttachmentRecovery();
  }

  const filename = choosePromptAttachmentFilename(baseline, current);
  return {
    shouldRecover: true,
    filename,
    recoveryPrompt: buildPromptAttachmentRecoveryPrompt(filename),
  };
}

export function choosePromptAttachmentFilename(
  baseline: ComposerAttachmentState,
  current: ComposerAttachmentState,
): string | null {
  const newNames = newComposerAttachmentNames(baseline, current);
  const txtNames = newNames.filter((name) => /\.txt$/i.test(name));
  if (txtNames.length === 1) {
    return txtNames[0] ?? null;
  }
  if (txtNames.length > 1) {
    return null;
  }
  return newNames.length === 1 ? (newNames[0] ?? null) : null;
}

export function newComposerAttachmentNames(
  baseline: ComposerAttachmentState,
  current: ComposerAttachmentState,
): string[] {
  const baselineNames = new Set(
    baseline.normalizedNames.map((name) => name.toLowerCase()),
  );
  const seen = new Set<string>();
  const output: string[] = [];
  for (const name of current.visibleNames) {
    const normalized = name.toLowerCase();
    if (!normalized || baselineNames.has(normalized) || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    output.push(name);
  }
  return output;
}

export function newComposerAttachmentCount(
  baseline: ComposerAttachmentState,
  current: ComposerAttachmentState,
): number {
  const newNames = newComposerAttachmentNames(baseline, current);
  if (newNames.length > 0) {
    return newNames.length;
  }
  return Math.max(0, current.visibleCount - baseline.visibleCount);
}

export async function recoverPromptAttachmentUploadIfNeeded(
  page: Page,
  baseline: ComposerAttachmentState,
  timeoutAt: number,
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = Math.min(
    startedAt + PROMPT_ATTACHMENT_RECOVERY_OBSERVE_MAX_MS,
    Math.max(startedAt + 1, timeoutAt - 1),
  );

  while (Date.now() < deadline) {
    const observation = await readPromptAttachmentObservation(page);
    if (!observation) {
      return false;
    }

    const decision = evaluatePromptAttachmentRecovery({
      baseline,
      current: observation.attachments,
      submitState: observation.submitState,
    });
    if (decision.shouldRecover && decision.recoveryPrompt) {
      await fillComposer(page, decision.recoveryPrompt);
      await waitForSubmitReadyAfterUploads(
        page,
        Math.min(
          submitReadyAfterUploadTimeoutMs(),
          Math.max(1, timeoutAt - Date.now()),
        ),
      );
      return true;
    }

    if (
      Date.now() - startedAt >= PROMPT_ATTACHMENT_RECOVERY_OBSERVE_MIN_MS &&
      !observation.attachments.attachmentsProcessing
    ) {
      return false;
    }

    await sleep(
      Math.min(
        adaptivePollIntervalMs("edge", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  return false;
}

async function readPromptAttachmentObservation(page: Page): Promise<{
  attachments: ComposerAttachmentState;
  submitState: SubmitStateSnapshot;
} | null> {
  const [attachments, submitState] = await Promise.all([
    readComposerAttachmentState(page).catch(() => null),
    callPageAdapter<SubmitStateSnapshot>(page, "readSubmitState").catch(
      () => null,
    ),
  ]);
  if (!attachments || !submitState) {
    return null;
  }
  return { attachments, submitState };
}

function noPromptAttachmentRecovery(): PromptAttachmentRecoveryDecision {
  return {
    shouldRecover: false,
    filename: null,
    recoveryPrompt: null,
  };
}
