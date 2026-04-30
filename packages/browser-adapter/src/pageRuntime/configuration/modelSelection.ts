import type { Locator, Page } from "playwright-core";

import type { CapabilityReportPayload } from "../../contracts";
import { INTERACTION_TIMEOUT_MS } from "../../timing";

import { clickWithInteractionJitter } from "../common";

import { findVisibleEffortTrigger } from "./effortSelection";
import {
  closeOpenComposerMenus,
  findVisibleInlineChoicePoint,
  findVisiblePopupChoicePoint,
  selectedChoiceMatches,
  waitForVisibleLocator,
} from "./shared";

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
    Math.min(timeoutMs, 250),
  );
  if (composerRightTrigger) {
    return composerRightTrigger;
  }

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
  const preferredTestIds =
    modelClass === "instant"
      ? ["model-switcher-gpt-5-3-instant", preferredTestId]
      : [preferredTestId];
  const preferredOption = await waitForVisibleLocator(
    page.locator(
      preferredTestIds.map((testId) => `[data-testid='${testId}']`).join(", "),
    ),
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
    page.locator(
      preferredTestIds.map((testId) => `[data-testid='${testId}']`).join(", "),
    ),
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
