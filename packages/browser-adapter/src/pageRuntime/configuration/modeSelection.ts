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
