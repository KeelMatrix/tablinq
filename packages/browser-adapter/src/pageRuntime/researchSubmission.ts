import type { Page } from "playwright-core";

import type { ResearchSubmissionStateSnapshot } from "../snapshotState";
import {
  normalizePromptText,
  normalizeResearchSubmissionState,
} from "../snapshotState";
import { adaptivePollIntervalMs } from "../timing";

import { callPageAdapter } from "./adapter";
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
  await callPageAdapter<boolean>(page, "clickSubmit");
  if (
    await waitForResearchSubmissionStarted(page, query, baseline, timeoutMs)
  ) {
    return readResearchSubmissionState(page);
  }
  if (await waitForResearchSubmissionStarted(page, query, baseline, 250)) {
    return readResearchSubmissionState(page);
  }

  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await focusComposer(page);
  await pressShortcutWithInteractionJitter(page, `${modifier}+Enter`);
  const submitted = await waitForResearchSubmissionStarted(
    page,
    query,
    baseline,
    timeoutMs,
  );
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

async function waitForResearchSubmissionStarted(
  page: Page,
  query: string,
  baseline: ResearchSubmissionStateSnapshot,
  timeoutMs: number,
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  const expected = normalizePromptText(query);

  while (Date.now() < deadline) {
    const submitted = await callPageAdapter<boolean>(
      page,
      "researchSubmissionStarted",
      { baseline, prompt: query },
    ).catch(() => false);
    if (submitted) {
      return true;
    }
    const current = await readResearchSubmissionState(page).catch(() => null);
    if (
      current &&
      current.submitted &&
      normalizePromptText(current.composerText) === expected
    ) {
      return true;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("edge", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  return callPageAdapter<boolean>(page, "researchSubmissionStarted", {
    baseline,
    prompt: query,
  }).catch(() => false);
}
