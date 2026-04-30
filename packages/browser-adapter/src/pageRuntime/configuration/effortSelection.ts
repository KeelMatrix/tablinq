import type { Locator, Page } from "playwright-core";

import type { CapabilityReportPayload } from "../../contracts";
import { INTERACTION_TIMEOUT_MS } from "../../timing";

import { clickWithInteractionJitter } from "../common";

import {
  closeOpenComposerMenus,
  findVisibleInlineChoicePoint,
  selectedChoiceMatches,
  waitForVisibleLocator,
} from "./shared";

export async function ensureConfiguredEffort(
  page: Page,
  report: CapabilityReportPayload,
  effort: "none" | "standard" | "extended",
): Promise<void> {
  if (effort === "none") {
    return;
  }

  const aliasTokens = [effort, ...(report.aliases.efforts[effort] ?? [])];
  if (await visibleComposerEffortMatches(page, effort)) {
    return;
  }

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

  let option = await findVisibleEffortChoicePoint(page, effort, 750);
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
  if (trigger) {
    await clickWithInteractionJitter(
      trigger,
      "picker_trigger",
      undefined,
      page,
    );
    option = await findVisibleEffortChoicePoint(page, effort);
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
  }

  if (await openNestedThinkingEffortMenu(page)) {
    option = await findVisibleEffortChoicePoint(page, effort);
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
  }

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

  throw new Error(
    `ui drift: could not find a concrete control for effort=${effort} after alias discovery`,
  );
}

export async function ensureConfiguredThinkingEffort(
  page: Page,
  _report: CapabilityReportPayload,
  effort: "standard" | "extended",
): Promise<boolean> {
  if (
    (await visibleComposerModelMatchesThinking(page)) &&
    (await visibleComposerEffortMatches(page, effort))
  ) {
    return true;
  }

  const directThinkingOption = await findVisibleThinkingChoicePoint(
    page,
    effort,
    750,
  );
  if (directThinkingOption) {
    if (directThinkingOption.checked !== "true") {
      await clickWithInteractionJitter(
        directThinkingOption.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
      await page.waitForTimeout(150);
    }
    await closeOpenComposerMenus(page);
    return true;
  }

  const trigger = await findVisibleModelPickerTriggerForEffort(page);
  if (!trigger) {
    return false;
  }

  await clickWithInteractionJitter(trigger, "picker_trigger", undefined, page);

  const openedThinkingOption = await findVisibleThinkingChoicePoint(
    page,
    effort,
    1_000,
  );
  if (openedThinkingOption) {
    if (openedThinkingOption.checked !== "true") {
      await clickWithInteractionJitter(
        openedThinkingOption.locator,
        "choice_select",
        {
          force: true,
        },
        page,
      );
      await page.waitForTimeout(150);
    }
    await closeOpenComposerMenus(page);
    return true;
  }

  const nestedTrigger = await findVisibleNestedThinkingEffortTrigger(page, 500);
  if (nestedTrigger) {
    await clickWithInteractionJitter(
      nestedTrigger,
      "picker_trigger",
      { force: true },
      page,
    );
    const option = await findVisibleEffortChoicePoint(page, effort, 1_000);
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
      return true;
    }
  }

  await closeOpenComposerMenus(page);
  return false;
}

export async function trySelectNestedThinkingEffort(
  page: Page,
  effort: "standard" | "extended",
): Promise<boolean> {
  if (await selectedChoiceMatches(page, [effort])) {
    return false;
  }

  if (await visibleComposerEffortMatches(page, effort)) {
    return false;
  }

  if (!(await openNestedThinkingEffortMenu(page))) {
    return false;
  }

  const option = await findVisibleEffortChoicePoint(page, effort, 1_000);
  if (!option) {
    return false;
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
    await page.waitForTimeout(150);
  }
  return true;
}

export async function findVisibleEffortTrigger(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort']",
        "[data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
        "#thinking-effort-trigger",
        "button[aria-controls*='effort']",
        "button[aria-label*='thinking'][aria-haspopup]",
        "button[aria-label*='effort'][aria-haspopup]",
      ].join(", "),
    ),
    timeoutMs,
  );
}

async function openNestedThinkingEffortMenu(page: Page): Promise<boolean> {
  let nestedTrigger = await findVisibleNestedThinkingEffortTrigger(page, 250);
  if (!nestedTrigger) {
    const modelTrigger = await findVisibleModelPickerTriggerForEffort(page);
    if (!modelTrigger) {
      return false;
    }
    await clickWithInteractionJitter(
      modelTrigger,
      "picker_trigger",
      undefined,
      page,
    );
    nestedTrigger = await findVisibleNestedThinkingEffortTrigger(page, 1_000);
  }
  if (!nestedTrigger) {
    return false;
  }
  if ((await nestedTrigger.count().catch(() => 0)) === 0) {
    return false;
  }
  await clickWithInteractionJitter(
    nestedTrigger,
    "picker_trigger",
    { force: true },
    page,
  );
  return true;
}

async function findVisibleModelPickerTriggerForEffort(
  page: Page,
): Promise<Locator | null> {
  const composerRightTrigger = await waitForVisibleLocator(
    page.locator(
      [
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Instant')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Thinking')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Legacy')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('GPT')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.3')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.4')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.5')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Standard')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Extended')",
      ].join(", "),
    ),
    250,
  );
  if (composerRightTrigger) {
    return composerRightTrigger;
  }
  return waitForVisibleLocator(
    page.locator(
      "button[aria-label='Model selector'], button[aria-label='Switch model'], button[data-testid='model-switcher-dropdown-button']",
    ),
    1_000,
  );
}

async function findVisibleNestedThinkingEffortTrigger(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort'], [data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
    ),
    timeoutMs,
  );
}

async function visibleComposerEffortMatches(
  page: Page,
  effort: "standard" | "extended",
): Promise<boolean> {
  return page
    .evaluate((requestedEffort: "standard" | "extended") => {
      const isVisible = (element: Element) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }
        const style = window.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0" &&
          element.getClientRects().length > 0
        );
      };
      return Array.from(
        document.querySelectorAll(
          "#thinking-effort-trigger, button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
        ),
      ).some((element) => {
        if (!isVisible(element)) {
          return false;
        }
        const text = (element.textContent ?? "")
          .replace(/\\s+/g, " ")
          .trim()
          .toLowerCase();
        if (requestedEffort === "standard") {
          return (
            text.includes("standard") ||
            (text.includes("thinking") && !text.includes("extended"))
          );
        }
        return text.includes("extended");
      });
    }, effort)
    .catch(() => false);
}

async function visibleComposerModelMatchesThinking(
  page: Page,
): Promise<boolean> {
  return page
    .evaluate(() => {
      const isVisible = (element: Element) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }
        const style = window.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0" &&
          element.getClientRects().length > 0
        );
      };
      return Array.from(
        document.querySelectorAll(
          "#thinking-effort-trigger, button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
        ),
      ).some((element) => {
        if (!isVisible(element)) {
          return false;
        }
        const text = (element.textContent ?? "")
          .replace(/\\s+/g, " ")
          .trim()
          .toLowerCase();
        return (
          text.includes("thinking") ||
          text.includes("standard") ||
          text.includes("extended") ||
          text.includes("5.4") ||
          text.includes("5.5")
        );
      });
    })
    .catch(() => false);
}

async function findVisibleThinkingChoicePoint(
  page: Page,
  effort: "standard" | "extended",
  timeoutMs: number,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const choices = page.locator(
    "[role='menuitemradio']:visible, [role='option']:visible",
  );
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const count = await choices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = choices.nth(index);
      const testId = (
        (await locator.getAttribute("data-testid").catch(() => null)) ?? ""
      ).toLowerCase();
      const text = ((await locator.textContent().catch(() => null)) ?? "")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      const isThinking =
        testId.includes("thinking") || text.includes("thinking");
      if (!isThinking) {
        continue;
      }
      const hasNestedEffort =
        (await locator
          .locator(
            "[data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
          )
          .count()
          .catch(() => 0)) > 0;
      const matchesEffort =
        effort === "standard"
          ? text.includes("standard")
          : text.includes("extended");
      if (!matchesEffort) {
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

async function findVisibleEffortChoicePoint(
  page: Page,
  token: string,
  timeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const popupChoices = page.locator(
    "[role='menuitemradio']:visible, [role='option']:visible",
  );
  const deadline = Date.now() + timeoutMs;
  const expectedToken = token.toLowerCase();
  while (Date.now() < deadline) {
    const count = await popupChoices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = popupChoices.nth(index);
      const testId = (await locator.getAttribute("data-testid")) ?? "";
      if (testId.startsWith("model-switcher-")) {
        continue;
      }
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
