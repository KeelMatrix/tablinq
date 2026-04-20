import { createHash } from "node:crypto";
import { mkdir, readFile, stat } from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";
import type { Download, Frame, Locator, Page } from "playwright-core";

import {
  chooseUniqueArtifactPath,
  contentTypeForFile,
} from "./artifacts";
import { loadPageAdapterSource } from "./assets";
import { safePageUrl } from "./capabilityProbe";
import type {
  ArtifactMetadataSnapshot,
  AskExecutionCommand,
  CapabilityReportPayload,
} from "./contracts";
import {
  DEEP_RESEARCH_START_CONFIRM_TIMEOUT_MS,
  DOWNLOAD_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
  NAVIGATION_TIMEOUT_MS,
  SNAPSHOT_POLL_INTERVAL_MS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
  SUBMIT_START_TIMEOUT_MS,
  UPLOAD_ACK_TIMEOUT_MS,
  deepResearchStartDelayMs,
} from "./timing";
import {
  AskBaseline,
  AskSnapshot,
  DeepResearchFrameSnapshot,
  ResearchSnapshot,
  ResearchSubmissionStateSnapshot,
  SubmitStateSnapshot,
  mergeResearchSnapshots,
  normalizeAskBaseline,
  normalizeAskSnapshot,
  normalizeDeepResearchFrameSnapshot,
  normalizePromptText,
  normalizeResearchSnapshot,
  normalizeResearchSubmissionState,
  researchSubmissionStateToSnapshot,
  scoreDeepResearchFrame,
} from "./snapshotState";
import { deepResearchPathname, resolveConversationUrl } from "./urls";

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

export async function callPageAdapter<T>(
  page: Page,
  methodName:
    | "composerReady"
    | "fillComposer"
    | "clickSubmit"
    | "applyAskConfiguration"
    | "uploadAcknowledged"
    | "submitReady"
    | "attachmentsStillProcessing"
    | "readSubmitState"
    | "readAskBaseline"
    | "readAskSnapshot"
    | "readResearchSubmissionState"
    | "readResearchSnapshot"
    | "submitStarted"
    | "researchSubmissionStarted",
  payload?: unknown,
): Promise<T> {
  return page.evaluate(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      if (typeof method !== "function") {
        throw new Error(`page adapter method not found: ${methodName}`);
      }
      return method(payload);
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
  ) as Promise<T>;
}

export async function waitForPageAdapterCondition(
  page: Page,
  methodName:
    | "composerReady"
    | "uploadAcknowledged"
    | "submitReady"
    | "attachmentsStillProcessing"
    | "submitStarted"
    | "researchSubmissionStarted",
  payload: unknown,
  timeoutMs: number,
): Promise<void> {
  await page.waitForFunction(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      return typeof method === "function" ? Boolean(method(payload)) : false;
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
    {
      timeout: timeoutMs,
    },
  );
}

export async function waitForSubmitReadyAfterUploads(
  page: Page,
  timeoutMs = SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
): Promise<void> {
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
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
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
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const started = await callPageAdapter<boolean>(page, "submitStarted", {
      baseline,
    }).catch(() => false);
    if (started) {
      return true;
    }
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }
  return callPageAdapter<boolean>(page, "submitStarted", { baseline }).catch(
    () => false,
  );
}

export async function waitForInteractionSurface(page: Page): Promise<void> {
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
    INTERACTION_TIMEOUT_MS,
  );
}

export async function fillComposer(page: Page, prompt: string): Promise<void> {
  await waitForInteractionSurface(page);
  await callPageAdapter(page, "fillComposer", { prompt });
}

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
  await fillComposer(page, query);
  const deadline = Date.now() + timeoutMs;
  const expected = normalizePromptText(query);

  while (Date.now() < deadline) {
    const state = await readResearchSubmissionState(page).catch(() => null);
    if (state && normalizePromptText(state.composerText) === expected) {
      return state;
    }
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
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
  if (normalizePromptText(baseline.composerText) !== normalizePromptText(query)) {
    throw new Error(
      "research submit failed: query was not present in the composer before submission",
    );
  }

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
  await page.keyboard.press(`${modifier}+Enter`).catch(() => undefined);
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
  if (normalizePromptText(current.composerText) !== normalizePromptText(query)) {
    throw new Error(
      "research submit failed: query was not present in the composer before submission",
    );
  }
  if (current.onDeepResearchLanding) {
    throw new Error(
      "research submit failed: query was filled on /deep-research but research never started",
    );
  }
  throw new Error("research submit failed: query was filled but research never started");
}

export async function beginDeepResearchRun(
  page: Page,
  timeoutMs: number,
): Promise<ResearchSnapshot | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frameSnapshot = await readDeepResearchFrameSnapshot(page);
    if (frameSnapshot?.startVisible) {
      if (!frameSnapshot.startEnabled) {
        await sleep(SNAPSHOT_POLL_INTERVAL_MS);
        continue;
      }
      await sleep(
        Math.min(
          deepResearchStartDelayMs(),
          Math.max(0, deadline - Date.now()),
        ),
      );
      if (!(await clickDeepResearchStart(page))) {
        throw new Error(
          "research start failed: Deep Research Start control was present but not actionable",
        );
      }
      const started = await waitForDeepResearchStartConfirmation(
        page,
        Math.min(
          DEEP_RESEARCH_START_CONFIRM_TIMEOUT_MS,
          Math.max(1, deadline - Date.now()),
        ),
      );
      if (started) {
        return started;
      }
      throw new Error(
        "research start failed: Deep Research Start was clicked but the job never began",
      );
    }

    const submission = await readResearchSubmissionState(page).catch(() => null);
    if (
      submission &&
      !submission.onDeepResearchLanding &&
      submission.status !== "idle"
    ) {
      return researchSubmissionStateToSnapshot(submission);
    }

    const merged = await readLiveResearchSnapshot(page).catch(() => null);
    if (
      merged &&
      merged.status !== "idle" &&
      merged.statusMessage !== "awaiting start confirmation"
    ) {
      return merged;
    }

    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }

  throw new Error(
    "research start failed: Deep Research planner never became actionable",
  );
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
    await composer.first().focus().catch(() => undefined);
  }
}

export async function applyAskConfiguration(
  page: Page,
  command: AskExecutionCommand,
  report: CapabilityReportPayload | null,
): Promise<Page> {
  await waitForInteractionSurface(page);
  if (!command.mode || command.mode === "normal" || command.mode === "search") {
    await ensureComposerModelPickerAvailable(page);
  }
  if (
    !command.modelClass &&
    !command.effort &&
    (!command.mode || command.mode === "normal")
  ) {
    return page;
  }

  if (!report) {
    throw new Error(
      "capability missing: no capability report is available for model or mode selection",
    );
  }

  if (command.modelClass) {
    await ensureConfiguredModelClass(page, report, command.modelClass);
  }
  if (command.effort) {
    await ensureConfiguredEffort(page, report, command.effort);
  }
  if (command.mode && command.mode !== "normal") {
    page = await ensureConfiguredMode(page, report, command.mode);
  }
  return page;
}

export async function ensureComposerModelPickerAvailable(
  page: Page,
): Promise<void> {
  let clearedAny = false;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const chip = await findVisibleComposerChipRemoveButton(page, 1_000);
    if (!chip) {
      break;
    }
    clearedAny = true;
    await chip.click();
    await page.waitForTimeout(150);
  }

  if (await findVisibleModelPickerTrigger(page, 1_000)) {
    return;
  }

  if (clearedAny) {
    throw new Error(
      "ui drift: could not restore the normal composer after clearing active chips before ask submission",
    );
  }
}

export async function findVisibleModelPickerTrigger(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      "button[aria-label='Model selector'], button[aria-label='Switch model'], button[data-testid='model-switcher-dropdown-button']",
    ),
    timeoutMs,
  );
}

export async function findVisibleComposerChipRemoveButton(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "[data-testid='composer-root'] button[aria-label*='click to remove' i]",
        "[data-testid='composer-root'] button[aria-label*='remove' i]",
        "form button[aria-label*='click to remove' i]",
        "form button[aria-label*='remove' i]",
      ].join(", "),
    ),
    timeoutMs,
  );
}

export async function waitForVisibleLocator(
  locator: Locator,
  timeoutMs: number,
): Promise<Locator | null> {
  const candidate = locator.first();
  try {
    await candidate.waitFor({ state: "visible", timeout: timeoutMs });
    return candidate;
  } catch {
    return null;
  }
}

export async function findVisiblePopupChoicePoint(
  page: Page,
  token: string,
  timeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const popupChoices = page.locator(
    "[role='menuitemradio']:visible, [role='menuitem']:visible, [role='option']:visible",
  );
  const deadline = Date.now() + timeoutMs;
  const expectedToken = token.toLowerCase();
  while (Date.now() < deadline) {
    const count = await popupChoices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = popupChoices.nth(index);
      const text = ((await locator.textContent()) ?? "")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (!text.includes(expectedToken)) {
        continue;
      }
      return {
        locator,
        checked: await locator.getAttribute("aria-checked"),
      };
    }
    await page.waitForTimeout(100);
  }
  return null;
}

async function readToggleState(locator: Locator): Promise<boolean> {
  const pressed = await locator.getAttribute("aria-pressed").catch(() => null);
  if (pressed !== null) {
    return pressed === "true";
  }
  const checked = await locator.getAttribute("aria-checked").catch(() => null);
  if (checked !== null) {
    return checked === "true";
  }
  return false;
}

async function findVisibleInlineChoicePoint(
  page: Page,
  selectors: string,
  tokens: readonly string[],
  timeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<{ locator: Locator; selected: boolean } | null> {
  const inlineChoices = page.locator(selectors);
  const deadline = Date.now() + timeoutMs;
  const expectedTokens = tokens
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  while (Date.now() < deadline) {
    const count = await inlineChoices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = inlineChoices.nth(index);
      if (!(await locator.isVisible().catch(() => false))) {
        continue;
      }
      const text = [
        (await locator.textContent().catch(() => null)) ?? "",
        (await locator.getAttribute("aria-label").catch(() => null)) ?? "",
      ]
        .join(" ")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (!expectedTokens.some((token) => text.includes(token))) {
        continue;
      }
      return {
        locator,
        selected: await readToggleState(locator),
      };
    }
    await page.waitForTimeout(100);
  }
  return null;
}

export async function ensureConfiguredModelClass(
  page: Page,
  report: CapabilityReportPayload,
  modelClass: "instant" | "thinking",
): Promise<void> {
  const aliasTokens = [
    modelClass,
    ...(report.aliases.model_classes[modelClass] ?? []),
  ];
  const inlineOption = await findVisibleInlineChoicePoint(
    page,
    [
      "button[data-model]",
      "[data-slot='model-picker'] button",
      "[role='group'] button",
    ].join(", "),
    aliasTokens,
    750,
  );
  if (inlineOption) {
    if (!inlineOption.selected) {
      await inlineOption.locator.click({ force: true });
      await page.waitForTimeout(150);
    }
    return;
  }

  const preferredTestId =
    modelClass === "instant"
      ? "model-switcher-gpt-5-3"
      : "model-switcher-gpt-5-4-thinking";
  const preferredOption = await waitForVisibleLocator(
    page.locator(`[data-testid='${preferredTestId}']`),
    1_000,
  );
  const option =
    preferredOption !== null
      ? {
          locator: preferredOption,
          checked: await preferredOption.getAttribute("aria-checked"),
        }
      : await findVisiblePopupChoicePoint(page, modelClass, 750);
  if (option) {
    if (option.checked !== "true") {
      await option.locator.click({ force: true });
      await page.waitForTimeout(150);
    }
    return;
  }

  const trigger = await findVisibleModelPickerTrigger(page, INTERACTION_TIMEOUT_MS);
  if (!trigger) {
    if (!report.matrix.model_picker_visible) {
      throw new Error(`capability missing: model picker; requested=${modelClass}`);
    }
    throw new Error(
      `ui drift: could not find a model selector trigger for model=${modelClass}`,
    );
  }

  await trigger.click();
  const reopenedPreferredOption = await waitForVisibleLocator(
    page.locator(`[data-testid='${preferredTestId}']`),
    1_000,
  );
  const reopenedOption =
    reopenedPreferredOption !== null
      ? {
          locator: reopenedPreferredOption,
          checked: await reopenedPreferredOption.getAttribute("aria-checked"),
        }
      : await findVisiblePopupChoicePoint(page, modelClass);
  if (!reopenedOption) {
    throw new Error(
      `ui drift: could not find a concrete control for model=${modelClass} after alias discovery`,
    );
  }
  if (reopenedOption.checked !== "true") {
    await reopenedOption.locator.click({ force: true });
    if (modelClass === "thinking") {
      await findVisibleEffortTrigger(page, 2_500);
    } else {
      await page.waitForTimeout(300);
    }
  }
}

export async function ensureConfiguredEffort(
  page: Page,
  report: CapabilityReportPayload,
  effort: "none" | "standard" | "extended",
): Promise<void> {
  if (effort === "none") {
    return;
  }

  const aliasTokens = [effort, ...(report.aliases.efforts[effort] ?? [])];
  const inlineOption = await findVisibleInlineChoicePoint(
    page,
    [
      "button[data-effort]",
      "[data-slot='effort-toggle'] button",
      "[role='group'] button",
    ].join(", "),
    aliasTokens,
    750,
  );
  if (inlineOption) {
    if (!inlineOption.selected) {
      await inlineOption.locator.click({ force: true });
      await page.waitForTimeout(150);
    }
    return;
  }

  let option = await findVisiblePopupChoicePoint(page, effort, 750);
  if (option) {
    if (option.checked !== "true") {
      await option.locator.click({ force: true });
    }
    return;
  }

  const trigger = await findVisibleEffortTrigger(page, INTERACTION_TIMEOUT_MS);
  if (!trigger) {
    if (!report.matrix.thinking_effort_visible) {
      throw new Error(`capability missing: thinking effort; requested=${effort}`);
    }
    throw new Error(
      `ui drift: could not find an effort trigger for effort=${effort}`,
    );
  }

  await trigger.click();
  option = await findVisiblePopupChoicePoint(page, effort);
  if (!option) {
    throw new Error(
      `ui drift: could not find a concrete control for effort=${effort} after alias discovery`,
    );
  }
  if (option.checked !== "true") {
    await option.locator.click({ force: true });
  }
}

export async function findVisibleEffortTrigger(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "button.__composer-pill[aria-haspopup]",
        "button[aria-controls*='effort']",
        "button[aria-label*='thinking'][aria-haspopup]",
      ].join(", "),
    ),
    timeoutMs,
  );
}

export async function ensureConfiguredMode(
  page: Page,
  report: CapabilityReportPayload,
  mode: "search" | "research",
): Promise<Page> {
  const visible =
    mode === "search"
      ? report.matrix.search_mode_visible
      : report.matrix.research_mode_visible;
  if (!visible) {
    if (mode === "research") {
      return await navigateDirectlyToDeepResearch(page, INTERACTION_TIMEOUT_MS);
    }
    throw new Error(`capability missing: ${mode} mode; requested=${mode}`);
  }

  const locator = await waitForVisibleLocator(
    page.locator(
      mode === "search"
        ? "[data-mode='search'], button[aria-label*='Search'], button[aria-label*='web']"
        : [
            "[data-mode='research']",
            "a[data-testid='deep-research-sidebar-item']",
            "a[href='/deep-research']",
            "a[href$='/deep-research']",
            "button[aria-label*='Research']",
          ].join(", "),
    ),
    INTERACTION_TIMEOUT_MS,
  );
  if (!locator) {
    throw new Error(`ui drift: could not find ${mode} mode control after probing it`);
  }
  const isActive =
    mode === "research"
      ? page.url().toLowerCase().includes("/deep-research")
      : (await locator.getAttribute("aria-pressed")) === "true";
  const controlHref =
    mode === "research" ? await locator.getAttribute("href").catch(() => null) : null;
  const requiresDeepResearchLandingWait =
    mode === "research" &&
    (await locator
      .evaluate((element) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }
        const href = element.getAttribute("href") ?? "";
        return (
          element.tagName.toLowerCase() === "a" ||
          element.getAttribute("data-testid") === "deep-research-sidebar-item" ||
          href.toLowerCase().includes("deep-research")
        );
      })
      .catch(() => false));
  if (!isActive) {
    if (requiresDeepResearchLandingWait) {
      await activateModeControl(locator);
      const activatedPage = await waitForDeepResearchLanding(
        page,
        INTERACTION_TIMEOUT_MS,
        {
          throwOnTimeout: false,
        },
      );
      if (activatedPage) {
        return activatedPage;
      }
      if (!controlHref) {
        throw new Error(
          "research mode activation failed: Deep Research control did not expose a href",
        );
      }
      const resolvedHref = new URL(controlHref, page.url()).toString();
      await page.evaluate((href) => {
        window.location.assign(href);
      }, resolvedHref);
    } else {
      await activateModeControl(locator);
    }
  }
  if (requiresDeepResearchLandingWait) {
    const landingPage = await waitForDeepResearchLanding(
      page,
      INTERACTION_TIMEOUT_MS,
    );
    if (!landingPage) {
      throw new Error(
        "research mode activation failed: Deep Research landing never became active",
      );
    }
    return landingPage;
  }
  if (mode === "research") {
    const landingPage = await waitForDeepResearchLanding(
      page,
      INTERACTION_TIMEOUT_MS,
      {
        throwOnTimeout: false,
      },
    );
    if (landingPage) {
      return landingPage;
    }
    return await navigateDirectlyToDeepResearch(page, INTERACTION_TIMEOUT_MS);
  }
  return page;
}

export async function navigateDirectlyToDeepResearch(
  page: Page,
  timeoutMs: number,
): Promise<Page> {
  const url = new URL(page.url());
  url.pathname = deepResearchPathname(url.pathname);
  await page.goto(url.toString(), { waitUntil: "domcontentloaded" });
  const landingPage = await waitForDeepResearchLanding(page, timeoutMs);
  if (!landingPage) {
    throw new Error(
      "research mode activation failed: Deep Research landing never became active",
    );
  }
  return landingPage;
}

export async function activateModeControl(locator: Locator): Promise<void> {
  try {
    await locator.click();
    return;
  } catch (error) {
    if (!(error instanceof Error) || !/intercepts pointer events/i.test(error.message)) {
      throw error;
    }
  }

  try {
    await locator.click({ force: true });
    return;
  } catch {
    await locator.evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        throw new Error("mode control is not actionable");
      }
      element.focus();
      element.click();
    });
  }
}

export async function candidateDeepResearchPages(page: Page): Promise<Page[]> {
  return [page, ...page.context().pages().filter((candidate) => candidate !== page).reverse()]
    .filter((candidate, index, all) => all.indexOf(candidate) === index)
    .filter((candidate) =>
      (safePageUrl(candidate)?.toLowerCase() ?? "").includes("/deep-research"),
    );
}

export async function deepResearchComposerVisible(page: Page): Promise<boolean> {
  return await page
    .evaluate((selectors: string[]) => {
      const isVisible = (element: Element | null) => {
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

      for (const selector of selectors) {
        const element = document.querySelector(selector);
        if (isVisible(element)) {
          return true;
        }
      }
      return false;
    }, [
      "#prompt-textarea",
      "#prompt-textarea[role='textbox']",
      "[role='textbox'][aria-label*='Chat with ChatGPT']",
      "textarea[placeholder='Get a detailed report']",
    ])
    .catch(() => false);
}

export async function waitForDeepResearchLanding(
  page: Page,
  timeoutMs: number,
  options: { throwOnTimeout?: boolean } = {},
): Promise<Page | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (const candidate of await candidateDeepResearchPages(page)) {
      await candidate.waitForLoadState("domcontentloaded").catch(() => undefined);
      const landingState = await readResearchSubmissionState(candidate).catch(
        () => null,
      );
      if (
        landingState?.onDeepResearchLanding ||
        landingState?.composerReady ||
        (await deepResearchComposerVisible(candidate)) ||
        (await readDeepResearchFrameSnapshot(candidate).catch(() => null)) !==
          null
      ) {
        return candidate;
      }
    }
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }
  if (options.throwOnTimeout === false) {
    return null;
  }
  throw new Error(
    "research mode activation failed: Deep Research landing never became active",
  );
}

export async function uploadAttachments(
  page: Page,
  files: string[],
  report: CapabilityReportPayload | null,
): Promise<void> {
  if (files.length === 0) {
    return;
  }

  const uploadSurface = page.locator(
    [
      "input[type='file']",
      "[data-slot='upload-entry']",
      "[data-testid='upload-button']",
      "button[aria-label*='Add photos']",
      "button[aria-label*='Add files']",
      "label[for]",
    ].join(", "),
  );
  if (
    !report?.matrix.upload_control_visible &&
    (await uploadSurface.count().catch(() => 0)) === 0
  ) {
    throw new Error(
      "upload failed: no upload control is available in the current browser session",
    );
  }

  const resolvedFiles = await Promise.all(
    files.map(async (filePath) => {
      const absolutePath = resolve(filePath);
      const fileStat = await stat(absolutePath).catch(() => null);
      if (!fileStat?.isFile()) {
        throw new Error(`upload failed: file not found: ${absolutePath}`);
      }
      return absolutePath;
    }),
  );

  const fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) > 0) {
    await fileInput.setInputFiles(resolvedFiles);
  } else {
    const uploadEntry = page
      .locator(
        [
          "[data-slot='upload-entry']",
          "[data-testid='upload-button']",
          "label[for]",
          "button[aria-label*='file']",
          "button[aria-label*='photo']",
        ].join(", "),
      )
      .first();
    if ((await uploadEntry.count()) === 0) {
      throw new Error("upload failed: upload entry point could not be found");
    }
    const chooserPromise = page.waitForEvent("filechooser", {
      timeout: INTERACTION_TIMEOUT_MS,
    });
    await uploadEntry.click();
    const chooser = await chooserPromise.catch(() => null);
    if (!chooser) {
      throw new Error("upload failed: file chooser did not open");
    }
    await chooser.setFiles(resolvedFiles);
  }

  const expectedNames = resolvedFiles.map((filePath) => basename(filePath));
  await waitForPageAdapterCondition(
    page,
    "uploadAcknowledged",
    {
      names: expectedNames,
    },
    UPLOAD_ACK_TIMEOUT_MS,
  ).catch(() => {
    throw new Error(
      `upload failed: the page did not acknowledge uploaded files: ${expectedNames.join(", ")}`,
    );
  });
}

export async function clickSubmit(page: Page): Promise<void> {
  await waitForInteractionSurface(page);
  const baseline = await callPageAdapter<SubmitStateSnapshot>(
    page,
    "readSubmitState",
  );
  const clicked = await callPageAdapter<boolean>(page, "clickSubmit");
  if (clicked && (await waitForSubmitStarted(page, baseline))) {
    return;
  }

  const clickedActiveComposerSubmit = await clickActiveComposerSubmit(page);
  if (
    clickedActiveComposerSubmit &&
    (await waitForSubmitStarted(page, baseline))
  ) {
    return;
  }

  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await focusComposer(page);
  await page.keyboard.press(`${modifier}+Enter`).catch(() => undefined);
  const submitted = await waitForSubmitStarted(page, baseline);
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
    const clicked = await candidate.click().then(() => true).catch(() => false);
    if (clicked) {
      return true;
    }
  }

  return false;
}

export async function readAskBaseline(page: Page): Promise<AskBaseline> {
  return normalizeAskBaseline(
    await callPageAdapter<AskBaseline>(page, "readAskBaseline"),
  );
}

export async function readAskSnapshot(
  page: Page,
  baseline?: AskBaseline | null,
): Promise<AskSnapshot> {
  return normalizeAskSnapshot(
    await callPageAdapter<AskSnapshot>(page, "readAskSnapshot", {
      baseline: baseline ?? null,
    }),
  );
}

export async function readLiveResearchSnapshot(
  page: Page,
): Promise<ResearchSnapshot> {
  const submission = await readResearchSubmissionState(page).catch(() => null);
  const domSnapshot = submission
    ? researchSubmissionStateToSnapshot(submission)
    : normalizeResearchSnapshot(
        await callPageAdapter<ResearchSnapshot>(page, "readResearchSnapshot"),
      );
  const frameSnapshot = await readDeepResearchFrameSnapshot(page);
  return mergeResearchSnapshots(domSnapshot, submission, frameSnapshot);
}

export async function waitForDeepResearchStartConfirmation(
  page: Page,
  timeoutMs: number,
): Promise<ResearchSnapshot | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frameSnapshot = await readDeepResearchFrameSnapshot(page);
    if (frameSnapshot?.startVisible) {
      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
      continue;
    }
    const snapshot = await readLiveResearchSnapshot(page).catch(() => null);
    if (
      snapshot &&
      snapshot.status !== "idle" &&
      snapshot.statusMessage !== "awaiting start confirmation"
    ) {
      return snapshot;
    }
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }
  return null;
}

export async function clickDeepResearchStart(page: Page): Promise<boolean> {
  for (const frame of deepResearchCandidateFrames(page)) {
    const button = frame.getByRole("button", { name: /^Start$/i }).first();
    const visible = await button.isVisible().catch(() => false);
    if (!visible) {
      continue;
    }
    const enabled = await button.isEnabled().catch(() => false);
    if (!enabled) {
      continue;
    }
    const clicked = await button
      .click({ force: true, timeout: INTERACTION_TIMEOUT_MS })
      .then(() => true)
      .catch(() => false);
    if (clicked) {
      return true;
    }
  }
  return false;
}

export function deepResearchCandidateFrames(page: Page): Frame[] {
  return page.frames().filter((frame) => frame !== page.mainFrame());
}

export async function readDeepResearchFrameSnapshot(
  page: Page,
): Promise<DeepResearchFrameSnapshot | null> {
  const snapshots: DeepResearchFrameSnapshot[] = [];
  for (const frame of deepResearchCandidateFrames(page)) {
    const snapshot = await inspectDeepResearchFrame(frame);
    if (snapshot) {
      snapshots.push(snapshot);
    }
  }
  if (snapshots.length === 0) {
    return null;
  }
  snapshots.sort(
    (left, right) =>
      scoreDeepResearchFrame(right) - scoreDeepResearchFrame(left),
  );
  return snapshots[0] ?? null;
}

export async function inspectDeepResearchFrame(
  frame: Frame,
): Promise<DeepResearchFrameSnapshot | null> {
  try {
    const snapshot = await frame.evaluate(() => {
      const normalize = (value: string | null | undefined): string =>
        String(value ?? "")
          .replace(/\u00a0/g, " ")
          .replace(/\r/g, "")
          .trim();
      const nestedRootFrame = document.querySelector("iframe#root");
      const nestedRootDocument =
        nestedRootFrame instanceof HTMLIFrameElement
          ? nestedRootFrame.contentDocument
          : null;
      const activeDocument =
        nestedRootDocument?.body && nestedRootDocument.body.childElementCount > 0
          ? nestedRootDocument
          : document;
      const activeWindow = activeDocument.defaultView ?? window;
      const HTMLElementCtor =
        activeWindow.HTMLElement ?? window.HTMLElement ?? HTMLElement;
      const isVisible = (element: Element): boolean => {
        if (!(element instanceof HTMLElementCtor) || element.hidden) {
          return false;
        }
        const style = activeWindow.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0"
        );
      };
      const buttonName = (element: Element): string =>
        normalize(
          element.getAttribute("aria-label") ??
            element.textContent ??
            element.getAttribute("title"),
        );
      const buttons = Array.from(
        activeDocument.querySelectorAll("button, [role='button']"),
      ).filter(isVisible);
      const buttonNames = buttons.map((element) =>
        buttonName(element).toLowerCase(),
      );
      const startButton =
        buttons.find((element) => /^start$/i.test(buttonName(element))) ?? null;
      const startVisible = Boolean(startButton);
      const startEnabled = startButton
        ? !(
            startButton.getAttribute("aria-disabled") === "true" ||
            startButton.getAttribute("data-disabled") === "true" ||
            ("disabled" in startButton &&
              Boolean(
                (
                  startButton as HTMLButtonElement | HTMLInputElement
                ).disabled,
              ))
          )
        : false;
      const hasPlannerControls =
        buttonNames.includes("edit") || buttonNames.includes("cancel");
      const stopResearchVisible = buttonNames.some(
        (name) => name === "stop" || name === "stop research",
      );
      const explicitReportRoot =
        activeDocument.querySelector("[data-deep-research-text]") ??
        activeDocument.querySelector("[class*='reportPage']") ??
        activeDocument.querySelector("article") ??
        activeDocument.querySelector("[role='article']") ??
        activeDocument.querySelector("[role='document']") ??
        activeDocument.querySelector("pre");
      const reportRoot =
        explicitReportRoot ?? activeDocument.querySelector("main");
      const statusRoot =
        activeDocument.querySelector("[data-deep-research-status]") ??
        activeDocument.querySelector("[class*='text-token-text-secondary']");
      const bodyText = normalize(
        reportRoot instanceof HTMLElementCtor
          ? reportRoot.innerText
          : activeDocument.body?.innerText ?? "",
      );
      const statusText = normalize(
        statusRoot instanceof HTMLElementCtor ? statusRoot.innerText : "",
      );
      const lowerUrl = window.location.href.toLowerCase();
      const lowerText = `${statusText}\n${bodyText}`.trim().toLowerCase();
      const starting =
        lowerText.includes("starting") ||
        lowerText.includes("getting ready") ||
        lowerText.includes("preparing");
      const generatingReport =
        lowerText.includes("providing a comprehensive report for the user");
      const plannerShell =
        hasPlannerControls ||
        lowerText.includes("proposed research plan") ||
        lowerText.includes("review the proposed research plan") ||
        lowerText.includes("review the research plan");
      const candidate =
        startVisible ||
        lowerUrl.includes("deep_research") ||
        lowerUrl.includes("oaiusercontent") ||
        lowerText.includes("deep research") ||
        stopResearchVisible ||
        lowerText.includes("gathering sources") ||
        lowerText.includes("drafting report") ||
        lowerText.includes("edit") ||
        lowerText.includes("cancel");
      if (!candidate) {
        return null;
      }
      const lines = bodyText
        .split(/\n+/)
        .map((line) => line.trim())
        .filter(Boolean);
      const waiting =
        lowerText.includes("gathering sources") ||
        lowerText.includes("collecting sources") ||
        lowerText.includes("searching") ||
        lowerText.includes("reviewing sources") ||
        lowerText.includes("forming a plan");
      const running =
        lowerText.includes("drafting report") ||
        lowerText.includes("writing report") ||
        lowerText.includes("analyzing") ||
        lowerText.includes("working") ||
        generatingReport ||
        stopResearchVisible;
      const failed =
        lowerText.includes("failed") ||
        lowerText.includes("error") ||
        lowerText.includes("retry");
      const provisionalShell =
        !explicitReportRoot &&
        (plannerShell ||
          starting ||
          waiting ||
          generatingReport ||
          stopResearchVisible);
      const reportText = startVisible || provisionalShell ? "" : bodyText;
      const completed =
        statusText.toLowerCase().includes("research completed") ||
        lowerText.includes("research completed");
      const phase: DeepResearchFrameSnapshot["phase"] = startVisible
        ? "planning"
        : plannerShell
          ? "planning"
          : failed
            ? "failed"
            : completed
              ? "done"
              : waiting
                ? "waiting_for_sources"
                : starting
                  ? "running"
                  : running
                    ? "running"
                    : reportText
                      ? "done"
                      : "running";
      return {
        phase,
        statusMessage: startVisible
          ? "awaiting start confirmation"
          : (statusText ||
              (starting ? "starting research" : lines[0] || null)),
        reportText,
        frameUrl: window.location.href,
        startVisible,
        startEnabled,
      };
    });
    return snapshot ? normalizeDeepResearchFrameSnapshot(snapshot) : null;
  } catch {
    return null;
  }
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

export async function collectArtifacts(
  page: Page,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot[]> {
  const handles = await page
    .locator(
      [
        "[data-slot='artifact-link']",
        "[data-testid='artifact-download']",
        "a[download]",
        "a[aria-label*='Download']",
        "button[aria-label*='Download']",
        "a:has-text('Download')",
        "button:has-text('Download')",
        "a:has-text('Export')",
        "button:has-text('Export')",
      ].join(", "),
    )
    .all();
  const artifacts: ArtifactMetadataSnapshot[] = [];

  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const artifactReady = await handle.evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const explicit =
        element.dataset.artifactReady ??
        element.getAttribute("data-artifact-ready");
      if (explicit === "false") {
        return false;
      }
      if (explicit === "true") {
        return true;
      }
      if (element instanceof HTMLAnchorElement) {
        return Boolean(element.hasAttribute("download") || element.href);
      }
      return !(
        element.getAttribute("aria-disabled") === "true" ||
        element.hasAttribute("disabled")
      );
    });
    if (!artifactReady) {
      continue;
    }

    const download = await clickAndCaptureDownload(page, handle).catch(
      (error) => {
        throw new Error(
          `download failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    );
    artifacts.push(
      await persistDownload(
        download,
        downloadsDir,
        conversationId,
        messageId,
        index,
      ),
    );
  }

  return artifacts;
}

export async function clickAndCaptureDownload(
  page: Page,
  handle: Locator,
): Promise<Download> {
  const downloadPromise = page.waitForEvent("download", {
    timeout: DOWNLOAD_TIMEOUT_MS,
  });
  await handle.click({ noWaitAfter: true, force: true });
  return downloadPromise;
}

export async function persistDownload(
  download: Download,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
  index: number,
): Promise<ArtifactMetadataSnapshot> {
  const suggestedName =
    download.suggestedFilename() || `artifact-${index + 1}.bin`;
  const storagePath = await chooseUniqueArtifactPath(
    downloadsDir,
    suggestedName,
  );
  await mkdir(dirname(storagePath), { recursive: true });
  await download.saveAs(storagePath);

  const fileBuffer = await readFile(storagePath);
  const fileStat = await stat(storagePath);
  const sha256 = createHash("sha256").update(fileBuffer).digest("hex");
  const artifactId = createHash("sha1")
    .update(
      `${conversationId ?? "no-conversation"}:${messageId ?? "no-message"}:${storagePath}`,
    )
    .digest("hex")
    .slice(0, 16);

  return {
    artifact_id: `artifact-${artifactId}`,
    conversation_id: conversationId,
    message_id: messageId,
    file_name: basename(storagePath),
    content_type: contentTypeForFile(storagePath),
    size_bytes: fileStat.size,
    sha256,
    storage_path: storagePath,
  };
}
