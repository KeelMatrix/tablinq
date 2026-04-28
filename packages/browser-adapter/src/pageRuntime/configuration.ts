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
  if (!command.mode || command.mode === "normal") {
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
  if (command.mode && command.mode !== "normal" && command.mode !== "search") {
    await closeOpenComposerMenus(page);
    await waitForInteractionSurface(page);
    await page.waitForTimeout(750);
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

export async function findVisibleComposerModelChipRemoveButton(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "[data-testid='composer-root'] button[aria-label*='remove' i][aria-label*='think' i]",
        "[data-testid='composer-root'] button[aria-label*='remove' i][aria-label*='reason' i]",
        "[data-testid='composer-root'] button[aria-label*='remove' i][aria-label*='standard' i]",
        "[data-testid='composer-root'] button[aria-label*='remove' i][aria-label*='extended' i]",
        "form button[aria-label*='remove' i][aria-label*='think' i]",
        "form button[aria-label*='remove' i][aria-label*='reason' i]",
        "form button[aria-label*='remove' i][aria-label*='standard' i]",
        "form button[aria-label*='remove' i][aria-label*='extended' i]",
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

async function visibleComposerModelChipPresent(page: Page): Promise<boolean> {
  return page
    .evaluate<boolean>(
      `(() => {
      const isVisible = (element) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }
        if (element.hidden || element.getAttribute("aria-hidden") === "true") {
          return false;
        }
        const style = window.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0"
        );
      };
      const textFor = (element) =>
        [element.textContent ?? "", element.getAttribute("aria-label") ?? ""]
          .join(" ")
          .replace(/\\s+/g, " ")
          .trim()
          .toLowerCase();
      const composerRoots = Array.from(
        document.querySelectorAll("[data-testid='composer-root'], form"),
      );
      return composerRoots.some((root) =>
        Array.from(
          root.querySelectorAll(
            "button.__composer-pill, button[aria-haspopup], [role='button']",
          ),
        ).some((element) => {
          if (!isVisible(element)) {
            return false;
          }
          const text = textFor(element);
          if (
            text.includes("search") ||
            text.includes("research") ||
            text.includes("canvas")
          ) {
            return false;
          }
          return (
            text.includes("think") ||
            text.includes("reason") ||
            text.includes("standard") ||
            text.includes("extended")
          );
        }),
      );
    })()`,
    )
    .catch(() => false);
}

async function clearVisibleComposerModelChip(page: Page): Promise<boolean> {
  const chip = await findVisibleComposerModelChipRemoveButton(page, 250);
  if (!chip) {
    return false;
  }
  await clickWithInteractionJitter(chip, "chip_clear");
  await page.waitForTimeout(150);
  await closeOpenComposerMenus(page);
  return true;
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
    .evaluate<boolean>(
      `(() => {
      const tokensToMatch = ${JSON.stringify(expectedTokens)};
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
          .replace(/\\s+/g, " ")
          .trim()
          .toLowerCase();
        if (tokensToMatch.some((token) => text.includes(token))) {
          return true;
        }
      }
      return false;
    })()`,
    )
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
  if (modelClass === "instant") {
    if (await clearVisibleComposerModelChip(page)) {
      return;
    }
  }

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
  if (
    modelClass === "instant" &&
    !(await visibleComposerModelChipPresent(page))
  ) {
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
    await closeOpenComposerMenus(page);
    return;
  }

  let trigger = await findVisibleModelPickerTrigger(
    page,
    INTERACTION_TIMEOUT_MS,
  );
  if (!trigger) {
    await ensureComposerModelPickerAvailable(page);
    trigger = await findVisibleModelPickerTrigger(page, 1_000);
  }
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
  await closeOpenComposerMenus(page);
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
    await closeOpenComposerMenus(page);
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
  await closeOpenComposerMenus(page);
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
  if (mode === "search") {
    const searchAliases = report.aliases.modes.search ?? [];
    if (
      report.matrix.search_mode_visible &&
      searchAliases.some((alias) =>
        alias.toLowerCase().includes("click to remove"),
      )
    ) {
      return page;
    }
    await ensureConfiguredSearchMode(page);
    return page;
  }

  const visible = report.matrix.research_mode_visible;
  if (!visible) {
    return await navigateDirectlyToDeepResearch(page, INTERACTION_TIMEOUT_MS);
  }

  const locator = await waitForVisibleLocator(
    page.locator(
      [
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
  const isActive = page.url().toLowerCase().includes("/deep-research");
  const controlHref = await locator.getAttribute("href").catch(() => null);
  const requiresDeepResearchLandingWait = await locator
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
    .catch(() => false);
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

export async function ensureConfiguredSearchMode(page: Page): Promise<void> {
  await page.bringToFront().catch(() => undefined);
  if (await searchModeActive(page)) {
    return;
  }
  await activateSearchModeOnce(page, "locator");
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return;
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return;
  }

  const retried = await activateSearchModeOnce(page, "locator");
  if (retried === "already-active") {
    return;
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return;
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return;
  }

  const mouseRetried = await activateSearchModeOnce(page, "mouse");
  if (mouseRetried === "already-active") {
    return;
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return;
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return;
  }

  const keyboardRetried = await activateSearchModeOnce(page, "keyboard-enter");
  if (keyboardRetried === "already-active") {
    return;
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return;
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return;
  }

  const keyboardSpaceRetried = await activateSearchModeOnce(
    page,
    "keyboard-space",
  );
  if (keyboardSpaceRetried === "already-active") {
    return;
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return;
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return;
  }

  throw new Error(
    "ui drift: search mode activation did not show the Search chip",
  );
}

async function activateSearchModeOnce(
  page: Page,
  method: "locator" | "mouse" | "keyboard-enter" | "keyboard-space",
): Promise<"clicked" | "already-active"> {
  if (await searchModeActive(page)) {
    return "already-active";
  }
  await closeOpenComposerMenus(page);

  const trigger = await findVisibleComposerToolsTrigger(page, 1_500);
  if (!trigger) {
    throw new Error("capability missing: search mode");
  }

  await trigger.click();
  await page.waitForTimeout(500);
  const option = await findVisibleComposerSearchMenuItem(page, 2_500);
  if (!option) {
    await page.keyboard.press("Escape").catch(() => undefined);
    throw new Error("capability missing: search mode");
  }
  if (option.checked === "true" && (await searchModeChipActive(page))) {
    return "already-active";
  }

  try {
    if (method === "mouse") {
      const box = await option.locator.boundingBox();
      if (!box) {
        throw new Error("search mode control is not actionable");
      }
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    } else if (method === "keyboard-enter" || method === "keyboard-space") {
      await option.locator.focus();
      await page.keyboard.press(
        method === "keyboard-enter" ? "Enter" : "Space",
      );
    } else {
      await option.locator.click();
    }
    return "clicked";
  } catch (error) {
    throw new Error(
      `ui drift: could not activate search mode control: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
}

async function closeOpenComposerMenus(page: Page): Promise<void> {
  const openMenu = page.locator(
    [
      "[role='menu']:visible",
      "[role='listbox']:visible",
      "[data-radix-popper-content-wrapper] [role='menu']:visible",
      "[data-radix-popper-content-wrapper] [role='listbox']:visible",
    ].join(", "),
  );
  if ((await openMenu.count().catch(() => 0)) === 0) {
    return;
  }
  await page.keyboard.press("Escape").catch(() => undefined);
  await page.waitForTimeout(300);
}

async function searchModeActive(page: Page): Promise<boolean> {
  return searchModeChipActive(page);
}

async function searchModeChipActive(page: Page): Promise<boolean> {
  return page
    .evaluate<boolean>(
      `(() => {
      const isVisible = (element) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }
        if (element.hidden || element.getAttribute("aria-hidden") === "true") {
          return false;
        }
        const style = window.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0"
        );
      };
      return Array.from(
        document.querySelectorAll(
          [
            "button.__composer-pill",
            "button[aria-label*='Search']",
            "[data-mode='search'][aria-pressed='true']",
            "[data-mode='search'][data-selected='true']",
          ].join(", "),
        ),
      ).some((element) => {
        if (!isVisible(element)) {
          return false;
        }
        const normalize = (value) =>
          (value ?? "").replace(/\\s+/g, " ").trim().toLowerCase();
        const name = normalize(
          (element.textContent ?? "") +
            " " +
            (element.getAttribute("aria-label") ?? ""),
        );
        const inComposer =
          element.closest("[data-testid='composer-root']") !== null ||
          element.closest("form") !== null;
        return (
          inComposer &&
          /\\bsearch\\b/.test(name) &&
          !/\\bsearch chats\\b/.test(name) &&
          (name.includes("click to remove") ||
            element.classList.contains("__composer-pill") ||
            element.getAttribute("aria-pressed") === "true" ||
            element.getAttribute("data-selected") === "true")
        );
      });
    })()`,
    )
    .catch(() => false);
}

async function waitForStableSearchChip(
  page: Page,
  timeoutMs: number,
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await searchModeChipActive(page)) {
      await page.waitForTimeout(150);
      return searchModeChipActive(page);
    }
    await page.waitForTimeout(100);
  }
  return searchModeChipActive(page);
}

async function findVisibleComposerToolsTrigger(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  const direct = await waitForVisibleLocator(
    page.locator(
      [
        "#composer-plus-btn",
        "[data-testid='composer-plus-btn']",
        "[data-testid='composer-root'] button[aria-label*='Add files' i][aria-haspopup]",
        "[data-testid='composer-root'] button[aria-label*='Add photos' i][aria-haspopup]",
        "form button[aria-label*='Add files' i][aria-haspopup]",
        "form button[aria-label*='Add photos' i][aria-haspopup]",
        "form button[aria-label*='tools' i][aria-haspopup]",
        "form button[aria-label*='more' i][aria-haspopup]",
      ].join(", "),
    ),
    timeoutMs,
  );
  if (direct) {
    return direct;
  }
  return null;
}

async function findVisibleComposerSearchMenuItem(
  page: Page,
  timeoutMs: number,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const exactWebSearch = await waitForVisibleLocator(
    page
      .locator("[role='menu'] [role='menuitemradio']")
      .filter({ hasText: /^Web search$/i }),
    Math.min(timeoutMs, 1_000),
  );
  if (exactWebSearch) {
    return {
      locator: exactWebSearch,
      checked: await exactWebSearch.getAttribute("aria-checked"),
    };
  }

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const item = await visibleComposerSearchMenuItem(page);
    if (item) {
      return item;
    }
    await page.waitForTimeout(100);
  }
  return visibleComposerSearchMenuItem(page);
}

async function visibleComposerSearchMenuItem(
  page: Page,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const menuItems = page.locator(
    [
      "[role='menu'] [role='menuitemradio']",
      "[role='menu'] [role='menuitem']",
      "[data-radix-popper-content-wrapper] [role='menuitemradio']",
      "[data-radix-popper-content-wrapper] [role='menuitem']",
    ].join(", "),
  );
  const count = await menuItems.count().catch(() => 0);
  for (let index = 0; index < count; index += 1) {
    const item = menuItems.nth(index);
    if (!(await item.isVisible().catch(() => false))) {
      continue;
    }
    const text = [
      (await item.textContent().catch(() => null)) ?? "",
      (await item.getAttribute("aria-label").catch(() => null)) ?? "",
    ]
      .join(" ")
      .replace(/\s+/g, " ")
      .trim()
      .toLowerCase();
    if (
      (text === "search" ||
        text === "web search" ||
        text.includes("web search")) &&
      !text.includes("search chats")
    ) {
      return {
        locator: item,
        checked: await item.getAttribute("aria-checked"),
      };
    }
  }
  return null;
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
