import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  CapabilityReportPayload,
} from "../../contracts";

import { waitForInteractionSurface } from "../composer";

import { ensureConfiguredEffort } from "./effortSelection";
import { ensureConfiguredMode } from "./modeSelection";
import {
  ensureComposerModelPickerAvailable,
  ensureConfiguredModelClass,
} from "./modelSelection";
import { closeOpenComposerMenus } from "./shared";

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

  if (command.mode && command.mode !== "normal" && command.mode !== "search") {
    await closeOpenComposerMenus(page);
    await waitForInteractionSurface(page);
    await page.waitForTimeout(750);
    page = await ensureConfiguredMode(page, report, command.mode);
  }
  if (command.modelClass) {
    await ensureConfiguredModelClass(page, report, command.modelClass);
  }
  if (command.effort) {
    await ensureConfiguredEffort(page, report, command.effort);
  }
  return page;
}

export {
  ensureConfiguredEffort,
  findVisibleEffortTrigger,
} from "./effortSelection";

export {
  activateModeControl,
  ensureConfiguredMode,
  ensureConfiguredSearchMode,
  navigateDirectlyToDeepResearch,
} from "./modeSelection";

export {
  ensureComposerModelPickerAvailable,
  ensureConfiguredModelClass,
  findVisibleComposerChipRemoveButton,
  findVisibleModelPickerTrigger,
} from "./modelSelection";

export { findVisiblePopupChoicePoint, waitForVisibleLocator } from "./shared";
