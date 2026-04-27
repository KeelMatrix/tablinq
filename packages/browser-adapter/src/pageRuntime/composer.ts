import type { Page } from "playwright-core";

import type { SubmitStateSnapshot } from "../snapshotState";
import {
  adaptivePollIntervalMs,
  INTERACTION_TIMEOUT_MS,
  NAVIGATION_TIMEOUT_MS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
  SUBMIT_START_TIMEOUT_MS,
} from "../timing";

import { callPageAdapter, waitForPageAdapterCondition } from "./adapter";
import {
  clickWithInteractionJitter,
  pressShortcutWithInteractionJitter,
  prepareHighRiskInteraction,
  sleep,
} from "./common";

export async function waitForSubmitReadyAfterUploads(
  page: Page,
  timeoutMs = SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
): Promise<void> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  let state: SubmitStateSnapshot | null = null;

  while (Date.now() < deadline) {
    state = await callPageAdapter<SubmitStateSnapshot>(
      page,
      "readSubmitState",
    ).catch(() => null);
    if (
      state &&
      !state.attachmentsProcessing &&
      ((state.submitVisible && state.submitEnabled && !state.submitBusy) ||
        (!state.submitVisible && state.keyboardReady))
    ) {
      return;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("upload", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  state =
    state ??
    (await callPageAdapter<SubmitStateSnapshot>(page, "readSubmitState").catch(
      () => null,
    ));
  if (state?.attachmentsProcessing) {
    throw new Error(
      `submit failed: upload was acknowledged but attachments are still processing after ${timeoutMs}ms`,
    );
  }
  throw new Error(
    `submit failed: upload was acknowledged but submit never became ready after ${timeoutMs}ms`,
  );
}

export async function waitForSubmitStarted(
  page: Page,
  baseline: SubmitStateSnapshot,
  timeoutMs = SUBMIT_START_TIMEOUT_MS,
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const started = await callPageAdapter<boolean>(page, "submitStarted", {
      baseline,
    }).catch(() => false);
    if (started) {
      return true;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("edge", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  return callPageAdapter<boolean>(page, "submitStarted", { baseline }).catch(
    () => false,
  );
}

async function submitEdgeDetected(
  page: Page,
  baseline: SubmitStateSnapshot,
  timeoutMs = SUBMIT_START_TIMEOUT_MS,
): Promise<boolean> {
  return waitForSubmitStarted(page, baseline, timeoutMs);
}

type InteractionSurfaceOptions = {
  composerTimeoutMs?: number;
  reloadOnUnavailable?: boolean;
};

export async function waitForInteractionSurface(
  page: Page,
  options: InteractionSurfaceOptions = {},
): Promise<void> {
  try {
    await waitForInteractionSurfaceOnce(page, options.composerTimeoutMs);
    return;
  } catch (error) {
    if (!options.reloadOnUnavailable) {
      throw error;
    }
    if (await providerRequestInProgress(page)) {
      throw error;
    }
    await page
      .reload({ waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS })
      .catch(() => {
        throw error;
      });
    await waitForInteractionSurfaceOnce(page, options.composerTimeoutMs);
  }
}

async function waitForInteractionSurfaceOnce(
  page: Page,
  composerTimeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<void> {
  await page.waitForLoadState("domcontentloaded", {
    timeout: NAVIGATION_TIMEOUT_MS,
  });
  await page
    .waitForLoadState("networkidle", {
      timeout: 2_000,
    })
    .catch(() => undefined);
  await waitForPageAdapterCondition(
    page,
    "composerReady",
    undefined,
    composerTimeoutMs,
  );
}

async function providerRequestInProgress(page: Page): Promise<boolean> {
  const state = await callPageAdapter<SubmitStateSnapshot>(
    page,
    "readSubmitState",
  ).catch(() => null);
  if (!state) {
    return false;
  }
  return (
    state.stopVisible ||
    state.submitBusy ||
    !["idle", "done"].includes(state.streamState ?? "idle")
  );
}

export async function fillComposer(page: Page, prompt: string): Promise<void> {
  await waitForInteractionSurface(page);
  await callPageAdapter(page, "fillComposer", { prompt });
}

export async function focusComposer(page: Page): Promise<void> {
  const composer = page.locator(
    [
      "#prompt-textarea",
      "[data-slot='composer']",
      "[data-testid='composer-root'] textarea",
      "[data-testid='composer-root'] [contenteditable='true']",
      "textarea[aria-label*='Message']",
      "textarea[placeholder*='message']",
      "[role='textbox'][aria-label*='Message']",
      "[contenteditable='true'][aria-label*='Message']",
      "textarea",
      "[role='textbox']",
      "[contenteditable='true']",
    ].join(", "),
  );
  if ((await composer.count()) > 0) {
    const first = composer.first();
    const alreadyActive = await first
      .evaluate((element) => {
        const active = document.activeElement;
        return (
          active === element ||
          (element instanceof HTMLElement && element.contains(active))
        );
      })
      .catch(() => false);
    if (!alreadyActive) {
      await first.focus().catch(() => undefined);
    }
  }
}

export async function clickSubmit(page: Page): Promise<void> {
  await waitForInteractionSurface(page);
  const baseline = await callPageAdapter<SubmitStateSnapshot>(
    page,
    "readSubmitState",
  );
  await prepareHighRiskInteraction(page, "final_submit");
  await callPageAdapter<boolean>(page, "clickSubmit");
  if (await submitEdgeDetected(page, baseline)) {
    return;
  }

  const clickedActiveComposerSubmit = await clickActiveComposerSubmit(page);
  if (
    clickedActiveComposerSubmit &&
    (await submitEdgeDetected(page, baseline))
  ) {
    return;
  }
  if (await submitEdgeDetected(page, baseline, 250)) {
    return;
  }

  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await focusComposer(page);
  await pressShortcutWithInteractionJitter(page, `${modifier}+Enter`);
  const submitted = await submitEdgeDetected(page, baseline);
  if (!submitted) {
    throw new Error(
      "submit failed: prompt was not sent after clicking submit controls or using the keyboard shortcut",
    );
  }
}

export async function clickActiveComposerSubmit(page: Page): Promise<boolean> {
  const candidates = page.locator(
    [
      "[data-testid='composer-root'] [data-testid='send-button']",
      "[data-testid='composer-root'] button[aria-label='Send prompt']",
      "[data-testid='composer-root'] button[aria-label='Submit prompt']",
      "[data-testid='composer-root'] input[type='submit']",
      "#composer-submit-button",
      "[data-testid='send-button']",
      "button[aria-label='Send prompt']",
      "button[aria-label='Submit prompt']",
      "input[type='submit']",
    ].join(", "),
  );

  const count = await candidates.count();
  for (let index = 0; index < count; index += 1) {
    const candidate = candidates.nth(index);
    const visible = await candidate.isVisible().catch(() => false);
    if (!visible) {
      continue;
    }
    const actionable = await candidate
      .evaluate((element) => {
        const ariaDisabled = element.getAttribute("aria-disabled") === "true";
        const dataDisabled = element.getAttribute("data-disabled") === "true";
        const hasPopup = element.getAttribute("aria-haspopup") !== null;
        const nativeDisabled =
          "disabled" in element &&
          Boolean((element as HTMLButtonElement | HTMLInputElement).disabled);
        return !ariaDisabled && !dataDisabled && !nativeDisabled && !hasPopup;
      })
      .catch(() => false);
    if (!actionable) {
      continue;
    }
    await prepareHighRiskInteraction(page, "final_submit", candidate);
    const clicked = await clickWithInteractionJitter(candidate, "button_click")
      .then(() => true)
      .catch(() => false);
    if (clicked) {
      return true;
    }
  }

  return false;
}
