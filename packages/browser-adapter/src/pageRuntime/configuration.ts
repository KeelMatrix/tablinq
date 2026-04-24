import type { Locator, Page } from "playwright-core";

import type {
  AskExecutionCommand,
  CapabilityReportPayload,
} from "../contracts";
import { INTERACTION_TIMEOUT_MS } from "../timing";
import { deepResearchPathname } from "../urls";

import { waitForInteractionSurface } from "./composer";
import { clickWithInteractionJitter, pauseForInteraction } from "./common";
import { waitForDeepResearchLanding } from "./deepResearch";

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
    await clickWithInteractionJitter(chip, "chip_clear");
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

async function selectedChoiceMatches(
  page: Page,
  tokens: readonly string[],
): Promise<boolean> {
  const expectedTokens = tokens
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  if (expectedTokens.length === 0) {
    return false;
  }

  return page
    .evaluate((tokensToMatch) => {
      const selected = Array.from(
        document.querySelectorAll(
          [
            "[aria-pressed='true']",
            "[aria-checked='true']",
            "[data-selected='true']",
            "[data-state='checked']",
          ].join(", "),
        ),
      );
      for (const element of selected) {
        const text = [
          element.textContent ?? "",
          element.getAttribute("aria-label") ?? "",
          element.getAttribute("data-model") ?? "",
          element.getAttribute("data-effort") ?? "",
          element.getAttribute("data-mode") ?? "",
        ]
          .join(" ")
          .replace(/\s+/g, " ")
          .trim()
          .toLowerCase();
        if (tokensToMatch.some((token) => text.includes(token))) {
          return true;
        }
      }
      return false;
    }, expectedTokens)
    .catch(() => false);
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
  if (await selectedChoiceMatches(page, aliasTokens)) {
    return;
  }
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
      await clickWithInteractionJitter(
        inlineOption.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
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
      await clickWithInteractionJitter(
        option.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
      await page.waitForTimeout(150);
    }
    return;
  }

  const trigger = await findVisibleModelPickerTrigger(
    page,
    INTERACTION_TIMEOUT_MS,
  );
  if (!trigger) {
    if (!report.matrix.model_picker_visible) {
      throw new Error(
        `capability missing: model picker; requested=${modelClass}`,
      );
    }
    throw new Error(
      `ui drift: could not find a model selector trigger for model=${modelClass}`,
    );
  }

  await clickWithInteractionJitter(trigger, "picker_trigger", undefined, page);
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
    await clickWithInteractionJitter(
      reopenedOption.locator,
      "choice_select",
      {
        force: true,
      },
      page,
    );
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
  if (await selectedChoiceMatches(page, aliasTokens)) {
    return;
  }
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
      await clickWithInteractionJitter(
        inlineOption.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
      await page.waitForTimeout(150);
    }
    return;
  }

  let option = await findVisiblePopupChoicePoint(page, effort, 750);
  if (option) {
    if (option.checked !== "true") {
      await clickWithInteractionJitter(
        option.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
    }
    return;
  }

  const trigger = await findVisibleEffortTrigger(page, INTERACTION_TIMEOUT_MS);
  if (!trigger) {
    if (!report.matrix.thinking_effort_visible) {
      throw new Error(
        `capability missing: thinking effort; requested=${effort}`,
      );
    }
    throw new Error(
      `ui drift: could not find an effort trigger for effort=${effort}`,
    );
  }

  await clickWithInteractionJitter(trigger, "picker_trigger", undefined, page);
  option = await findVisiblePopupChoicePoint(page, effort);
  if (!option) {
    throw new Error(
      `ui drift: could not find a concrete control for effort=${effort} after alias discovery`,
    );
  }
  if (option.checked !== "true") {
    await clickWithInteractionJitter(
      option.locator,
      "choice_select",
      {
        force: true,
      },
      page,
    );
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
    throw new Error(
      `ui drift: could not find ${mode} mode control after probing it`,
    );
  }
  const isActive =
    mode === "research"
      ? page.url().toLowerCase().includes("/deep-research")
      : (await locator.getAttribute("aria-pressed")) === "true";
  const controlHref =
    mode === "research"
      ? await locator.getAttribute("href").catch(() => null)
      : null;
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
          element.getAttribute("data-testid") ===
            "deep-research-sidebar-item" ||
          href.toLowerCase().includes("deep-research")
        );
      })
      .catch(() => false));
  if (!isActive) {
    if (requiresDeepResearchLandingWait) {
      await activateModeControl(page, locator);
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
      await activateModeControl(page, locator);
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
  await pauseForInteraction("mode_toggle", Math.max(0, timeoutMs));
  await page.goto(url.toString(), { waitUntil: "domcontentloaded" });
  const landingPage = await waitForDeepResearchLanding(page, timeoutMs);
  if (!landingPage) {
    throw new Error(
      "research mode activation failed: Deep Research landing never became active",
    );
  }
  return landingPage;
}

export async function activateModeControl(
  page: Page,
  locator: Locator,
): Promise<void> {
  try {
    await clickWithInteractionJitter(locator, "mode_toggle", undefined, page);
    return;
  } catch (error) {
    if (
      !(error instanceof Error) ||
      !/intercepts pointer events/i.test(error.message)
    ) {
      throw error;
    }
  }

  try {
    await clickWithInteractionJitter(
      locator,
      "mode_toggle",
      { force: true },
      page,
    );
    return;
  } catch {
    await pauseForInteraction("mode_toggle");
    await locator.evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        throw new Error("mode control is not actionable");
      }
      element.focus();
      element.click();
    });
  }
}
