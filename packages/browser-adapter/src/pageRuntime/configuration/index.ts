import type { Page } from "playwright-core";

import type { CapabilityReportPayload } from "../../contracts";

import { waitForInteractionSurface } from "../composer";

import {
  ensureConfiguredEffort,
  ensureConfiguredThinkingEffort,
  trySelectNestedThinkingEffort,
} from "./effortSelection";
import { ensureConfiguredMode } from "./modeSelection";
import {
  ensureComposerModelPickerAvailable,
  ensureConfiguredModelClass,
} from "./modelSelection";
import { closeOpenComposerMenus } from "./shared";

export type ConfiguredExecutionCommand = {
  mode?: "normal" | "search" | "research";
  modelClass?: "instant" | "thinking" | null;
  effort?: "none" | "standard" | "extended" | null;
};

export async function applyAskConfiguration(
  page: Page,
  command: ConfiguredExecutionCommand,
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

  if (command.mode && command.mode !== "normal") {
    await closeOpenComposerMenus(page);
    await waitForInteractionSurface(page);
    if (command.mode !== "search") {
      await page.waitForTimeout(750);
    }
    page = await ensureConfiguredMode(page, report, command.mode);
  }
  if (
    command.modelClass === "thinking" &&
    command.effort &&
    command.effort !== "none" &&
    (await ensureConfiguredThinkingEffort(page, report, command.effort))
  ) {
    await closeOpenComposerMenus(page);
    return page;
  }

  const selectedNestedThinkingEffort =
    command.modelClass === "thinking" &&
    command.effort &&
    command.effort !== "none" &&
    (await trySelectNestedThinkingEffort(page, command.effort));
  if (command.modelClass && !selectedNestedThinkingEffort) {
    await ensureConfiguredModelClass(page, report, command.modelClass);
  }
  if (command.effort && !selectedNestedThinkingEffort) {
    await ensureConfiguredEffort(page, report, command.effort);
  }
  await closeOpenComposerMenus(page);
  return page;
}

export {
  ensureConfiguredEffort,
  ensureConfiguredThinkingEffort,
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
