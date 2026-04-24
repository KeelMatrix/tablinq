import type { Page } from "playwright-core";

import type { ResearchSubmissionStateSnapshot } from "../snapshotState";
import {
  normalizePromptText,
  normalizeResearchSubmissionState,
} from "../snapshotState";
import { adaptivePollIntervalMs } from "../timing";

import { callPageAdapter, waitForPageAdapterCondition } from "./adapter";
import {
  fillComposer,
  focusComposer,
  waitForInteractionSurface,
} from "./composer";
import {
  pauseForInteraction,
  pressShortcutWithInteractionJitter,
  prepareHighRiskInteraction,
  sleep,
} from "./common";

export async function readResearchSubmissionState(
  page: Page,
): Promise<ResearchSubmissionStateSnapshot> {
  return normalizeResearchSubmissionState(
    await callPageAdapter<ResearchSubmissionStateSnapshot>(
      page,
      "readResearchSubmissionState",
    ),
  );
}

export async function ensureResearchComposerFilled(
  page: Page,
  query: string,
  timeoutMs: number,
): Promise<ResearchSubmissionStateSnapshot> {
  const startedAt = Date.now();
  await fillComposer(page, query);
  const deadline = Date.now() + timeoutMs;
  const expected = normalizePromptText(query);

  while (Date.now() < deadline) {
    const state = await readResearchSubmissionState(page).catch(() => null);
    if (state && normalizePromptText(state.composerText) === expected) {
      return state;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("edge", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  throw new Error(
    "research submit failed: query was not present in the composer before submission",
  );
}

export async function submitResearch(
  page: Page,
  query: string,
  timeoutMs: number,
): Promise<ResearchSubmissionStateSnapshot> {
  await waitForInteractionSurface(page);
  const baseline = await readResearchSubmissionState(page);
  if (
    normalizePromptText(baseline.composerText) !== normalizePromptText(query)
  ) {
    throw new Error(
      "research submit failed: query was not present in the composer before submission",
    );
  }

  await prepareHighRiskInteraction(page, "final_submit");
  await pauseForInteraction("button_click");
  const clicked = await callPageAdapter<boolean>(page, "clickSubmit");
  if (
    clicked &&
    (await waitForPageAdapterCondition(
      page,
      "researchSubmissionStarted",
      { baseline, prompt: query },
      timeoutMs,
    )
      .then(() => true)
      .catch(() => false))
  ) {
    return readResearchSubmissionState(page);
  }

  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await focusComposer(page);
  await pressShortcutWithInteractionJitter(page, `${modifier}+Enter`);
  const submitted = await waitForPageAdapterCondition(
    page,
    "researchSubmissionStarted",
    { baseline, prompt: query },
    timeoutMs,
  )
    .then(() => true)
    .catch(() => false);
  if (submitted) {
    return readResearchSubmissionState(page);
  }

  const current = await readResearchSubmissionState(page).catch(() => baseline);
  if (
    normalizePromptText(current.composerText) !== normalizePromptText(query)
  ) {
    throw new Error(
      "research submit failed: query was not present in the composer before submission",
    );
  }
  if (current.onDeepResearchLanding) {
    throw new Error(
      "research submit failed: query was filled on /deep-research but research never started",
    );
  }
  throw new Error(
    "research submit failed: query was filled but research never started",
  );
}
