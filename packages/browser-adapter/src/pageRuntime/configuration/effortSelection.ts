import type { Locator, Page } from "playwright-core";

import type { CapabilityReportPayload } from "../../contracts";
import { INTERACTION_TIMEOUT_MS } from "../../timing";

import { clickWithInteractionJitter } from "../common";

import {
  readVisibleComposerSelectionState,
  visibleSelectionMatchesEffort,
  visibleSelectionMatchesModelClass,
} from "./selectionState";
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

  if (await selectedChoiceMatches(page, aliasTokens, ["pro"])) {
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
    ["pro"],
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
    if (
      directThinkingOption.checked !== "true" ||
      !(await visibleComposerModelMatchesThinking(page))
    ) {
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
    if (
      openedThinkingOption.checked !== "true" ||
      !(await visibleComposerModelMatchesThinking(page))
    ) {
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
      if (
        option.checked !== "true" ||
        !(await visibleComposerModelMatchesThinking(page))
      ) {
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

  if (await selectThinkingEffortFromConfigureModal(page, effort)) {
    return true;
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
        // Legacy/current explicit labels stay first as stable fallbacks for older ChatGPT picker UIs.
        // Keep these even though the generic unlabeled composer-pill selector below handles newer labels.
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Instant')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Thinking')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Legacy')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('GPT')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.3')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.4')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('5.5')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Standard')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label]):has-text('Extended')",
        "button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
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

async function selectThinkingEffortFromConfigureModal(
  page: Page,
  effort: "standard" | "extended",
): Promise<boolean> {
  const configure = await waitForVisibleLocator(
    page
      .locator(
        "[data-testid='model-configure-modal']:visible, [role='menuitem']:visible",
      )
      .filter({ hasText: /configure/i }),
    500,
  );
  if (!configure) {
    return false;
  }

  await clickWithInteractionJitter(
    configure,
    "choice_select",
    { force: true },
    page,
  );
  const modal = await findVisibleIntelligenceModal(page, 1_500);
  if (!modal) {
    return false;
  }

  const thinkingRadio = await waitForVisibleLocator(
    modal.locator("[role='radio']:has-text('Thinking')"),
    1_000,
  );
  if (thinkingRadio) {
    const checked = await thinkingRadio.getAttribute("aria-checked");
    if (checked !== "true") {
      await clickWithInteractionJitter(
        thinkingRadio,
        "choice_select",
        {
          force: true,
        },
        page,
      );
      await page.waitForTimeout(150);
    }
  }

  const effortCombobox = await waitForVisibleLocator(
    modal
      .locator("[role='combobox'], button:visible")
      .filter({ hasText: /standard|extended/i }),
    1_000,
  );
  if (!effortCombobox) {
    await closeVisibleIntelligenceModal(page);
    return false;
  }

  const currentEffort = (
    (await effortCombobox.textContent().catch(() => null)) ?? ""
  )
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  if (!currentEffort.includes(effort)) {
    await clickWithInteractionJitter(
      effortCombobox,
      "picker_trigger",
      {
        force: true,
      },
      page,
    );
    const option = await findVisibleModalEffortOption(page, effort, 1_000);
    if (!option) {
      await closeVisibleIntelligenceModal(page);
      return false;
    }
    await clickWithInteractionJitter(
      option,
      "choice_select",
      {
        force: true,
      },
      page,
    );
    await page.waitForTimeout(150);
  }

  await closeVisibleIntelligenceModal(page);
  return true;
}

async function findVisibleIntelligenceModal(
  page: Page,
  timeoutMs: number,
): Promise<Locator | null> {
  return waitForVisibleLocator(
    page.locator(
      [
        "[data-testid='modal-intelligence-menu']",
        "[role='dialog']:has-text('Thinking effort')",
        "[role='dialog']:has-text('Intelligence')",
      ].join(", "),
    ),
    timeoutMs,
  );
}

async function findVisibleModalEffortOption(
  page: Page,
  effort: "standard" | "extended",
  timeoutMs: number,
): Promise<Locator | null> {
  const options = page.locator("[role='option']:visible");
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const count = await options.count();
    for (let index = 0; index < count; index += 1) {
      const locator = options.nth(index);
      const text = ((await locator.textContent().catch(() => null)) ?? "")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (text.includes(effort)) {
        return locator;
      }
    }
    await page.waitForTimeout(100);
  }
  return null;
}

async function closeVisibleIntelligenceModal(page: Page): Promise<void> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (!(await findVisibleIntelligenceModal(page, 100))) {
      return;
    }
    await page.keyboard.press("Escape").catch(() => undefined);
    await page.waitForTimeout(250);
  }
}

async function visibleComposerEffortMatches(
  page: Page,
  effort: "standard" | "extended",
): Promise<boolean> {
  const state = await readVisibleComposerSelectionState(page);
  return visibleSelectionMatchesEffort(state, effort);
}

async function visibleComposerModelMatchesThinking(
  page: Page,
): Promise<boolean> {
  const state = await readVisibleComposerSelectionState(page);
  return visibleSelectionMatchesModelClass(state, "thinking");
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
      if (!isThinking || testId.includes("pro") || text.includes("pro")) {
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
