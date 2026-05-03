import type { Locator, Page } from "playwright-core";

import type { CapabilityReportPayload } from "../../contracts";
import { INTERACTION_TIMEOUT_MS } from "../../timing";
import { deepResearchPathname } from "../../urls";

import { clickWithInteractionJitter, pauseForInteraction } from "../common";
import { waitForDeepResearchLanding } from "../deepResearch";

import { closeOpenComposerMenus, waitForVisibleLocator } from "./shared";

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

  const activeLanding = await waitForDeepResearchLanding(
    page,
    INTERACTION_TIMEOUT_MS,
    {
      throwOnTimeout: false,
    },
  );
  if (activeLanding) {
    return activeLanding;
  }

  try {
    return await navigateDirectlyToDeepResearch(page, INTERACTION_TIMEOUT_MS);
  } catch (directNavigationError) {
    const fallbackPage = await activateLegacyInlineResearchControl(
      page,
      report,
      directNavigationError,
    );
    if (fallbackPage) {
      return fallbackPage;
    }
    throw directNavigationError;
  }
}

async function activateLegacyInlineResearchControl(
  page: Page,
  report: CapabilityReportPayload,
  directNavigationError: unknown,
): Promise<Page | null> {
  const visible = report.matrix.research_mode_visible;
  if (!visible) {
    return null;
  }

  const locator = await waitForVisibleLocator(
    page.locator(
      ["[data-mode='research']", "button[aria-label*='Research']"].join(", "),
    ),
    Math.min(2_000, INTERACTION_TIMEOUT_MS),
  );
  if (!locator) {
    return null;
  }
  const isActive = page.url().toLowerCase().includes("/deep-research");
  const requiresDeepResearchLandingWait = await locator
    .evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      return element.tagName.toLowerCase() === "a";
    })
    .catch(() => false);
  if (!isActive) {
    await activateModeControl(page, locator);
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
  throw new Error(
    `research mode activation failed after direct navigation failed: ${
      directNavigationError instanceof Error
        ? directNavigationError.message
        : String(directNavigationError)
    }`,
  );
}

export async function ensureConfiguredSearchMode(
  page: Page,
): Promise<"already-active" | "activated"> {
  await page.bringToFront().catch(() => undefined);
  if (await searchModeActive(page)) {
    return "already-active";
  }
  await activateSearchModeOnce(page, "locator");
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return "activated";
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return "activated";
  }

  const retried = await activateSearchModeOnce(page, "locator");
  if (retried === "already-active") {
    return "already-active";
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return "activated";
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return "activated";
  }

  const mouseRetried = await activateSearchModeOnce(page, "mouse");
  if (mouseRetried === "already-active") {
    return "already-active";
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return "activated";
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return "activated";
  }

  const keyboardRetried = await activateSearchModeOnce(page, "keyboard-enter");
  if (keyboardRetried === "already-active") {
    return "already-active";
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return "activated";
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return "activated";
  }

  const keyboardSpaceRetried = await activateSearchModeOnce(
    page,
    "keyboard-space",
  );
  if (keyboardSpaceRetried === "already-active") {
    return "already-active";
  }
  await page.waitForTimeout(1_500);
  if (await waitForStableSearchChip(page, 10_000)) {
    return "activated";
  }
  await closeOpenComposerMenus(page);
  if (await searchModeActive(page)) {
    return "activated";
  }

  throw new Error(
    "ui drift: search mode activation did not show the Search chip",
  );
}

export async function clearConfiguredSearchMode(
  page: Page,
): Promise<"already-inactive" | "cleared"> {
  await page.bringToFront().catch(() => undefined);
  if (!(await searchModeActive(page))) {
    return "already-inactive";
  }

  const chip = await findVisibleSearchModeChip(page, 1_500);
  if (!chip) {
    throw new Error(
      "ui drift: search mode is active but no removable Search chip was found",
    );
  }
  await activateModeControl(page, chip);
  await page.waitForTimeout(500);
  if (await searchModeActive(page)) {
    throw new Error(
      "ui drift: Search chip remained active after clearing normal ask mode",
    );
  }
  return "cleared";
}

export async function clearIrrelevantComposerChips(
  page: Page,
): Promise<"already-clear" | "cleared"> {
  let clearedAny = false;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const chip = await findVisibleIrrelevantComposerChip(page, 250);
    if (!chip) {
      break;
    }
    clearedAny = true;
    await activateModeControl(page, chip);
    await page.waitForTimeout(150);
  }
  return clearedAny ? "cleared" : "already-clear";
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

async function searchModeActive(page: Page): Promise<boolean> {
  return searchModeChipActive(page);
}

async function findVisibleSearchModeChip(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "[data-testid='composer-root'] button.__composer-pill[aria-label*='Search' i][aria-label*='remove' i]",
        "[data-testid='composer-root'] button.__composer-pill[aria-label*='Search' i]",
        "[data-testid='composer-root'] button[aria-label*='Search' i][aria-label*='remove' i]",
        "[data-testid='composer-root'] button[aria-label*='Search' i][data-selected='true']",
        "form button.__composer-pill[aria-label*='Search' i][aria-label*='remove' i]",
        "form button.__composer-pill[aria-label*='Search' i]",
        "form button[aria-label*='Search' i][aria-label*='remove' i]",
        "form button[aria-label*='Search' i][data-selected='true']",
      ].join(", "),
    ),
    timeoutMs,
  );
}

async function findVisibleIrrelevantComposerChip(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const chips = page.locator(
      [
        "[data-testid='composer-root'] button[aria-label*='remove' i]",
        "[data-testid='composer-root'] button[aria-label*='click to remove' i]",
        "form button[aria-label*='remove' i]",
        "form button[aria-label*='click to remove' i]",
      ].join(", "),
    );
    const count = await chips.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const chip = chips.nth(index);
      if (!(await chip.isVisible().catch(() => false))) {
        continue;
      }
      const text = [
        (await chip.textContent().catch(() => null)) ?? "",
        (await chip.getAttribute("aria-label").catch(() => null)) ?? "",
      ]
        .join(" ")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (
        /\b(search|web search|research|deep research|canvas)\b/.test(text) &&
        !/\b(search chats)\b/.test(text)
      ) {
        return chip;
      }
    }
    await page.waitForTimeout(50);
  }
  return null;
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
        const webSearchChip = /\\bweb search\\b/.test(name);
        const activeSearchSignal =
          name.includes("click to remove") ||
          element.classList.contains("__composer-pill") ||
          element.getAttribute("aria-pressed") === "true" ||
          element.getAttribute("aria-checked") === "true" ||
          element.getAttribute("data-selected") === "true";
        return (
          inComposer &&
          /\\bsearch\\b/.test(name) &&
          !/\\bsearch chats\\b/.test(name) &&
          activeSearchSignal &&
          (!webSearchChip ||
            element.id === "search-chip" ||
            element.getAttribute("aria-pressed") === "true" ||
            element.getAttribute("aria-checked") === "true" ||
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
