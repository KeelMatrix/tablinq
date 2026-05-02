import type { Frame, Locator, Page } from "playwright-core";

import type {
  DeepResearchFrameSnapshot,
  ResearchSubmissionStateSnapshot,
  ResearchSnapshot,
} from "../snapshotState";
import {
  mergeResearchSnapshots,
  normalizeDeepResearchFrameSnapshot,
  normalizeResearchSnapshot,
  researchSubmissionStateToSnapshot,
  scoreDeepResearchFrame,
} from "../snapshotState";
import {
  adaptivePollIntervalMs,
  DEEP_RESEARCH_START_CONFIRM_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
} from "../timing";
import { safePageUrl } from "../capabilityProbe";

import { callPageAdapter } from "./adapter";
import { pauseForInteraction, sleep } from "./common";
import { readResearchSubmissionState } from "./researchSubmission";

const DEEP_RESEARCH_START_BUTTON_NAME = /^Start(?:\s+\d+)?$/i;
const DEEP_RESEARCH_START_BUTTON_TEXT = /^Start\b/i;
const DEEP_RESEARCH_START_BUTTON_SELECTOR = "button, [role='button']";
const DEEP_RESEARCH_START_CLICK_JITTER_MAX_MS = 250;
const COMPOSER_CONTROL_SELECTOR = [
  "[data-testid='composer-root']",
  "[data-testid='composer-footer-actions']",
  "[data-testid='composer-action-file-upload']",
  "[data-testid='send-button']",
  "#prompt-textarea",
  "textarea",
  "[role='textbox']",
].join(",");

interface NestedDeepResearchReport {
  reportText: string;
  statusText: string;
  href: string;
  phase: DeepResearchFrameSnapshot["phase"] | null;
  startVisible: boolean;
  startEnabled: boolean;
}

interface DeepResearchStartButtonState {
  visible: boolean;
  enabled: boolean;
}

export async function beginDeepResearchRun(
  page: Page,
  timeoutMs: number,
): Promise<ResearchSnapshot | null> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frameSnapshot = await readDeepResearchFrameSnapshot(page);
    if (frameSnapshot?.startVisible) {
      if (frameSnapshot.startVisible && !frameSnapshot.startEnabled) {
        await sleep(
          Math.min(
            adaptivePollIntervalMs("edge", Date.now() - startedAt),
            Math.max(1, deadline - Date.now()),
          ),
        );
        continue;
      }
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

    if (frameSnapshot?.phase === "planning") {
      await sleep(
        Math.min(
          adaptivePollIntervalMs("edge", Date.now() - startedAt),
          Math.max(1, deadline - Date.now()),
        ),
      );
      continue;
    }

    if (frameSnapshot) {
      const merged = await readLiveResearchSnapshot(page).catch(() => null);
      if (
        merged &&
        merged.status !== "idle" &&
        merged.statusMessage !== "awaiting start confirmation"
      ) {
        return merged;
      }
    }

    const shellPresent = await deepResearchShellPresent(page);
    const submission = await readResearchSubmissionState(page).catch(
      () => null,
    );
    if (
      !shellPresent &&
      submission &&
      !submission.onDeepResearchLanding &&
      submissionSnapshotIndicatesStarted(submission)
    ) {
      return researchSubmissionStateToSnapshot(submission);
    }

    await sleep(
      Math.min(
        adaptivePollIntervalMs("generation", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }

  throw new Error(
    "research start failed: Deep Research planner never became actionable",
  );
}

function submissionSnapshotIndicatesStarted(
  submission: ResearchSubmissionStateSnapshot,
): boolean {
  if (submission.status === "idle") {
    return false;
  }
  if (
    submission.status === "done" ||
    submission.status === "failed" ||
    submission.reportText.trim().length > 0
  ) {
    return true;
  }

  const statusMessage = (submission.statusMessage ?? "").trim().toLowerCase();
  if (!statusMessage || statusMessage === "awaiting start confirmation") {
    return false;
  }
  return (
    /^starting\.{3}$/i.test(statusMessage) ||
    statusMessage.includes("gathering sources") ||
    statusMessage.includes("collecting sources") ||
    statusMessage.includes("searching") ||
    statusMessage.includes("reviewing sources") ||
    statusMessage.includes("drafting report") ||
    statusMessage.includes("writing report") ||
    statusMessage.includes("analyzing") ||
    statusMessage.includes("working") ||
    statusMessage.includes("providing a comprehensive report")
  );
}

async function deepResearchShellPresent(page: Page): Promise<boolean> {
  if ((safePageUrl(page)?.toLowerCase() ?? "").includes("/deep-research")) {
    return true;
  }
  return page
    .evaluate(() =>
      Array.from(document.querySelectorAll("iframe")).some((frame) => {
        const haystack =
          `${frame.id} ${frame.title} ${frame.name} ${frame.src}`.toLowerCase();
        return (
          haystack.includes("deep-research") ||
          haystack.includes("deep_research") ||
          haystack.includes("oaiusercontent")
        );
      }),
    )
    .catch(() => false);
}

export async function candidateDeepResearchPages(page: Page): Promise<Page[]> {
  return [
    page,
    ...page
      .context()
      .pages()
      .filter((candidate) => candidate !== page)
      .reverse(),
  ]
    .filter((candidate, index, all) => all.indexOf(candidate) === index)
    .filter((candidate) =>
      (safePageUrl(candidate)?.toLowerCase() ?? "").includes("/deep-research"),
    );
}

export async function deepResearchComposerVisible(
  page: Page,
): Promise<boolean> {
  return await page
    .evaluate(
      (selectors: string[]) => {
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
      },
      [
        "#prompt-textarea",
        "#prompt-textarea[role='textbox']",
        "[role='textbox'][aria-label*='Chat with ChatGPT']",
        "textarea[placeholder='Get a detailed report']",
      ],
    )
    .catch(() => false);
}

export async function waitForDeepResearchLanding(
  page: Page,
  timeoutMs: number,
  options: { throwOnTimeout?: boolean } = {},
): Promise<Page | null> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (const candidate of await candidateDeepResearchPages(page)) {
      await candidate
        .waitForLoadState("domcontentloaded")
        .catch(() => undefined);
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
    await sleep(
      Math.min(
        adaptivePollIntervalMs("generation", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  if (options.throwOnTimeout === false) {
    return null;
  }
  throw new Error(
    "research mode activation failed: Deep Research landing never became active",
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
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frameSnapshot = await readDeepResearchFrameSnapshot(page);
    if (frameSnapshot?.startVisible) {
      await sleep(
        Math.min(
          adaptivePollIntervalMs("edge", Date.now() - startedAt),
          Math.max(1, deadline - Date.now()),
        ),
      );
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
    await sleep(
      Math.min(
        adaptivePollIntervalMs("generation", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  return null;
}

export async function clickDeepResearchStart(page: Page): Promise<boolean> {
  for (const frame of deepResearchCandidateFrames(page)) {
    const button = await findDeepResearchStartButton(frame);
    const enabled =
      (button ? await button.isEnabled().catch(() => false) : false) ||
      (await readDeepResearchStartButtonState(frame)).enabled;
    if (!enabled) {
      continue;
    }
    await pauseForInteraction(
      "button_click",
      DEEP_RESEARCH_START_CLICK_JITTER_MAX_MS,
    );
    if (
      frame !== page.mainFrame() &&
      (await clickDeepResearchStartButtonInFrame(frame))
    ) {
      return true;
    }
    const clicked = button
      ? await button
          .click({
            force: true,
            timeout: INTERACTION_TIMEOUT_MS,
          })
          .then(() => true)
          .catch(() => false)
      : false;
    if (clicked) {
      if (
        (await readDeepResearchStartButtonState(frame)).visible &&
        (await clickDeepResearchStartButtonInFrame(frame))
      ) {
        return true;
      }
      return true;
    }
    if (await clickDeepResearchStartButtonInFrame(frame)) {
      return true;
    }
  }
  return false;
}

async function findDeepResearchStartButton(
  frame: Frame,
): Promise<Locator | null> {
  const exactButton = frame
    .getByRole("button", { name: DEEP_RESEARCH_START_BUTTON_NAME })
    .first();
  if (await exactButton.isVisible().catch(() => false)) {
    return exactButton;
  }

  const broadButton = frame
    .locator(DEEP_RESEARCH_START_BUTTON_SELECTOR)
    .filter({ hasText: DEEP_RESEARCH_START_BUTTON_TEXT })
    .first();
  if (
    (await broadButton.count().catch(() => 0)) > 0 &&
    (await isDeepResearchStartButton(broadButton))
  ) {
    return broadButton;
  }
  return null;
}

async function isDeepResearchStartButton(button: Locator): Promise<boolean> {
  return button
    .evaluate(
      (element, selectors) => {
        if (!(element instanceof HTMLElement) || element.hidden) {
          return false;
        }
        const style = window.getComputedStyle(element);
        if (
          style.display === "none" ||
          style.visibility === "hidden" ||
          style.opacity === "0" ||
          element.getClientRects().length === 0 ||
          element.hasAttribute("disabled") ||
          element.getAttribute("aria-disabled") === "true"
        ) {
          return false;
        }
        const composerControlSelector = selectors.composerControlSelector;
        const normalize = (value: string | null | undefined) =>
          String(value ?? "")
            .replace(/\u00a0/g, " ")
            .replace(/\s+/g, " ")
            .trim();
        const hasPlanText = (value: string) => {
          const lower = value.toLowerCase();
          return (
            (lower.includes("edit") && lower.includes("cancel")) ||
            lower.includes("research plan") ||
            lower.includes("proposed research")
          );
        };
        const isComposerControl = (node: HTMLElement) => {
          if (node.closest(composerControlSelector)) {
            return true;
          }
          const form = node.closest("form");
          if (!form) {
            return false;
          }
          const formText = normalize(form.innerText || form.textContent);
          if (hasPlanText(formText)) {
            return false;
          }
          return Boolean(
            form.querySelector(selectors.composerSurfaceSelector) instanceof
            HTMLElement,
          );
        };
        const planRootFor = (node: HTMLElement): HTMLElement | null => {
          let current: HTMLElement | null = node.parentElement;
          for (let depth = 0; current && depth < 8; depth += 1) {
            const text = normalize(current.innerText || current.textContent);
            if (hasPlanText(text)) {
              return current;
            }
            current = current.parentElement;
          }
          return null;
        };
        if (isComposerControl(element)) {
          return false;
        }
        const label = normalize(element.innerText || element.textContent);
        const aria = normalize(element.getAttribute("aria-label"));
        const name = label || aria;
        if (!/^start\b/i.test(name)) {
          return false;
        }
        if (/dictation|microphone|record|voice/i.test(name + " " + aria)) {
          return false;
        }

        const contextRoot =
          planRootFor(element) ??
          element.closest("[data-deep-research-root]") ??
          element.closest("article") ??
          element.closest("section") ??
          element.parentElement;
        const contextText = normalize(
          contextRoot instanceof HTMLElement
            ? contextRoot.innerText || contextRoot.textContent
            : element.parentElement?.innerText,
        ).toLowerCase();
        return (
          (contextText.includes("edit") && contextText.includes("cancel")) ||
          contextText.includes("research plan") ||
          contextText.includes("proposed research")
        );
      },
      {
        composerControlSelector: COMPOSER_CONTROL_SELECTOR,
        composerSurfaceSelector:
          "[data-testid='composer-root'], #prompt-textarea, textarea, [role='textbox']",
      },
    )
    .catch(() => false);
}

async function readDeepResearchStartButtonState(
  frame: Frame,
): Promise<DeepResearchStartButtonState> {
  return frame
    .evaluate(
      (selectors) => {
        const normalize = (value: string | null | undefined) =>
          String(value ?? "")
            .replace(/\u00a0/g, " ")
            .replace(/\s+/g, " ")
            .trim();
        const documents = (): Document[] => {
          const nestedFrame = document.querySelector("iframe#root");
          const nestedDocument =
            nestedFrame instanceof HTMLIFrameElement
              ? nestedFrame.contentDocument
              : null;
          return nestedDocument ? [document, nestedDocument] : [document];
        };
        const isVisible = (element: Element): element is HTMLElement => {
          if (!(element instanceof HTMLElement) || element.hidden) {
            return false;
          }
          const style =
            element.ownerDocument.defaultView?.getComputedStyle(element);
          if (!style) {
            return false;
          }
          return (
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            style.opacity !== "0" &&
            element.getClientRects().length > 0
          );
        };
        const composerControlSelector = selectors.composerControlSelector;
        const hasPlanText = (value: string) => {
          const lower = value.toLowerCase();
          return (
            (lower.includes("edit") && lower.includes("cancel")) ||
            lower.includes("research plan") ||
            lower.includes("proposed research")
          );
        };
        const isComposerControl = (element: HTMLElement): boolean => {
          if (element.closest(composerControlSelector)) {
            return true;
          }
          const form = element.closest("form");
          if (!form) {
            return false;
          }
          const formText = normalize(form.innerText || form.textContent);
          if (hasPlanText(formText)) {
            return false;
          }
          return Boolean(
            form.querySelector(selectors.composerSurfaceSelector) instanceof
            HTMLElement,
          );
        };
        const planRootFor = (element: HTMLElement): HTMLElement | null => {
          let current: HTMLElement | null = element.parentElement;
          for (let depth = 0; current && depth < 8; depth += 1) {
            const text = normalize(current.innerText || current.textContent);
            if (hasPlanText(text)) {
              return current;
            }
            current = current.parentElement;
          }
          return null;
        };
        const isStartButton = (element: Element): element is HTMLElement => {
          if (!isVisible(element)) {
            return false;
          }
          if (isComposerControl(element)) {
            return false;
          }
          const label = normalize(element.innerText || element.textContent);
          const aria = normalize(element.getAttribute("aria-label"));
          const name = label || aria;
          if (!/^start\b/i.test(name)) {
            return false;
          }
          if (/dictation|microphone|record|voice/i.test(`${name} ${aria}`)) {
            return false;
          }
          const contextRoot =
            planRootFor(element) ??
            element.closest("[data-deep-research-root]") ??
            element.closest("article") ??
            element.closest("section") ??
            element.parentElement;
          const contextText = normalize(
            contextRoot instanceof HTMLElement
              ? contextRoot.innerText || contextRoot.textContent
              : element.parentElement?.innerText,
          ).toLowerCase();
          return (
            (contextText.includes("edit") && contextText.includes("cancel")) ||
            contextText.includes("research plan") ||
            contextText.includes("proposed research")
          );
        };
        let button: HTMLElement | undefined;
        for (const rootDocument of documents()) {
          button = Array.from(
            rootDocument.querySelectorAll("button, [role='button']"),
          ).find(isStartButton);
          if (button) {
            break;
          }
        }
        if (!(button instanceof HTMLElement)) {
          return { visible: false, enabled: false };
        }
        const nativeDisabled =
          "disabled" in button &&
          Boolean((button as HTMLButtonElement | HTMLInputElement).disabled);
        return {
          visible: true,
          enabled:
            !nativeDisabled &&
            !button.hasAttribute("disabled") &&
            button.getAttribute("aria-disabled") !== "true",
        };
      },
      {
        composerControlSelector: COMPOSER_CONTROL_SELECTOR,
        composerSurfaceSelector:
          "[data-testid='composer-root'], #prompt-textarea, textarea, [role='textbox']",
      },
    )
    .catch(() => ({ visible: false, enabled: false }));
}

async function clickDeepResearchStartButtonInFrame(
  frame: Frame,
): Promise<boolean> {
  return frame
    .evaluate(
      (selectors) => {
        const normalize = (value: string | null | undefined) =>
          String(value ?? "")
            .replace(/\u00a0/g, " ")
            .replace(/\s+/g, " ")
            .trim();
        const documents = (): Document[] => {
          const nestedFrame = document.querySelector("iframe#root");
          const nestedDocument =
            nestedFrame instanceof HTMLIFrameElement
              ? nestedFrame.contentDocument
              : null;
          return nestedDocument ? [document, nestedDocument] : [document];
        };
        const isVisible = (element: Element): element is HTMLElement => {
          if (!(element instanceof HTMLElement) || element.hidden) {
            return false;
          }
          const style =
            element.ownerDocument.defaultView?.getComputedStyle(element);
          if (!style) {
            return false;
          }
          return (
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            style.opacity !== "0" &&
            element.getClientRects().length > 0
          );
        };
        const composerControlSelector = selectors.composerControlSelector;
        const hasPlanText = (value: string) => {
          const lower = value.toLowerCase();
          return (
            (lower.includes("edit") && lower.includes("cancel")) ||
            lower.includes("research plan") ||
            lower.includes("proposed research")
          );
        };
        const isComposerControl = (element: HTMLElement): boolean => {
          if (element.closest(composerControlSelector)) {
            return true;
          }
          const form = element.closest("form");
          if (!form) {
            return false;
          }
          const formText = normalize(form.innerText || form.textContent);
          if (hasPlanText(formText)) {
            return false;
          }
          return Boolean(
            form.querySelector(selectors.composerSurfaceSelector) instanceof
            HTMLElement,
          );
        };
        const planRootFor = (element: HTMLElement): HTMLElement | null => {
          let current: HTMLElement | null = element.parentElement;
          for (let depth = 0; current && depth < 8; depth += 1) {
            const text = normalize(current.innerText || current.textContent);
            if (hasPlanText(text)) {
              return current;
            }
            current = current.parentElement;
          }
          return null;
        };
        const isStartButton = (element: Element): element is HTMLElement => {
          if (
            !isVisible(element) ||
            (element instanceof HTMLElement && isComposerControl(element))
          ) {
            return false;
          }
          const label = normalize(element.innerText || element.textContent);
          const aria = normalize(element.getAttribute("aria-label"));
          const name = label || aria;
          if (!/^start\b/i.test(name)) {
            return false;
          }
          if (/dictation|microphone|record|voice/i.test(`${name} ${aria}`)) {
            return false;
          }
          const contextRoot =
            planRootFor(element) ??
            element.closest("[data-deep-research-root]") ??
            element.closest("article") ??
            element.closest("section") ??
            element.parentElement;
          const contextText = normalize(
            contextRoot instanceof HTMLElement
              ? contextRoot.innerText || contextRoot.textContent
              : element.parentElement?.innerText,
          ).toLowerCase();
          return (
            (contextText.includes("edit") && contextText.includes("cancel")) ||
            contextText.includes("research plan") ||
            contextText.includes("proposed research")
          );
        };
        let button: HTMLElement | undefined;
        for (const rootDocument of documents()) {
          button = Array.from(
            rootDocument.querySelectorAll("button, [role='button']"),
          ).find(isStartButton);
          if (button) {
            break;
          }
        }
        if (!(button instanceof HTMLElement)) {
          return false;
        }
        const nativeDisabled =
          "disabled" in button &&
          Boolean((button as HTMLButtonElement | HTMLInputElement).disabled);
        if (
          nativeDisabled ||
          button.hasAttribute("disabled") ||
          button.getAttribute("aria-disabled") === "true"
        ) {
          return false;
        }
        button.click();
        return true;
      },
      {
        composerControlSelector: COMPOSER_CONTROL_SELECTOR,
        composerSurfaceSelector:
          "[data-testid='composer-root'], #prompt-textarea, textarea, [role='textbox']",
      },
    )
    .catch(() => false);
}

export function deepResearchCandidateFrames(page: Page): Frame[] {
  const childFrames = page
    .frames()
    .filter((frame) => frame !== page.mainFrame());
  return [page.mainFrame(), ...childFrames];
}

export async function readDeepResearchFrameSnapshot(
  page: Page,
): Promise<DeepResearchFrameSnapshot | null> {
  const snapshots: DeepResearchFrameSnapshot[] = [];
  for (const frame of deepResearchCandidateFrames(page)) {
    const nestedSnapshot = await readNestedDeepResearchFrameSnapshot(frame);
    if (nestedSnapshot) {
      snapshots.push(nestedSnapshot);
    }
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

async function readNestedDeepResearchFrameSnapshot(
  frame: Frame,
): Promise<DeepResearchFrameSnapshot | null> {
  const nestedReport = await frame
    .evaluate<NestedDeepResearchReport | null>(
      `(() => {
      const normalize = (value) => String(value ?? "")
        .replace(/\\u00a0/g, " ")
        .replace(/\\r/g, "")
        .trim();
      const nestedFrame = document.querySelector("iframe#root");
      const nestedDocument = nestedFrame && "contentDocument" in nestedFrame
        ? nestedFrame.contentDocument
        : null;
      if (!nestedDocument) {
        return null;
      }
      const reportRoot =
        nestedDocument.querySelector("[data-deep-research-text]") ??
        nestedDocument.querySelector("[class*='reportPage']") ??
        nestedDocument.querySelector("article, [role='article'], [role='document'], main, body");
      const statusRoot =
        nestedDocument.querySelector("[data-deep-research-status], [class*='text-token-text-secondary']") ??
        nestedDocument.body;
      const isVisible = (element) => {
        if (!(element instanceof HTMLElement) || element.hidden) {
          return false;
        }
        const style = nestedDocument.defaultView.getComputedStyle(element);
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.opacity !== "0" &&
          element.getClientRects().length > 0
        );
      };
      const isStartButton = (element) => {
        if (!isVisible(element)) {
          return false;
        }
        const label = normalize(element.innerText || element.textContent);
        const aria = normalize(element.getAttribute("aria-label"));
        const name = label || aria;
        if (!/^start\\b/i.test(name)) {
          return false;
        }
        if (/dictation|microphone|record|voice/i.test(name + " " + aria)) {
          return false;
        }
        return true;
      };
      const startButton = Array.from(
        nestedDocument.querySelectorAll("button, [role='button']")
      ).find(isStartButton);
      const nativeDisabled =
        startButton &&
        "disabled" in startButton &&
        Boolean(startButton.disabled);
      return {
        reportText: normalize(
          "innerText" in (reportRoot ?? {})
            ? reportRoot.innerText
            : reportRoot?.textContent
        ),
        statusText: normalize(
          "innerText" in (statusRoot ?? {})
            ? statusRoot.innerText
            : statusRoot?.textContent
        ),
        href: nestedDocument.location.href,
        phase: startButton ? "planning" : null,
        startVisible: Boolean(startButton),
        startEnabled: Boolean(
          startButton &&
          !nativeDisabled &&
          !startButton.hasAttribute("disabled") &&
          startButton.getAttribute("aria-disabled") !== "true"
        ),
      };
    })()`,
    )
    .catch(() => null);
  if (!nestedReport?.reportText && !nestedReport?.startVisible) {
    return null;
  }
  const lowerText =
    `${nestedReport.statusText}\n${nestedReport.reportText}`.toLowerCase();
  const phase: DeepResearchFrameSnapshot["phase"] =
    nestedReport.phase ??
    (lowerText.includes("research completed")
      ? "done"
      : lowerText.includes("gathering sources") ||
          lowerText.includes("collecting sources") ||
          lowerText.includes("searching") ||
          lowerText.includes("reviewing sources") ||
          lowerText.includes("forming a plan")
        ? "waiting_for_sources"
        : lowerText.includes("starting") ||
            lowerText.includes("getting ready") ||
            lowerText.includes("preparing")
          ? "running"
          : nestedReport.reportText
            ? "done"
            : "running");
  return normalizeDeepResearchFrameSnapshot({
    phase,
    statusMessage: nestedReport.startVisible
      ? "awaiting start confirmation"
      : nestedReport.statusText || null,
    reportText: nestedReport.startVisible ? "" : nestedReport.reportText,
    frameUrl: frame.url() || nestedReport.href,
    startVisible: nestedReport.startVisible,
    startEnabled: nestedReport.startEnabled,
  });
}

export async function inspectDeepResearchFrame(
  frame: Frame,
): Promise<DeepResearchFrameSnapshot | null> {
  const normalize = (value: string | null | undefined): string =>
    String(value ?? "")
      .replace(/\u00a0/g, " ")
      .replace(/\r/g, "")
      .trim();
  const visibleRole = async (name: RegExp): Promise<boolean> =>
    frame
      .getByRole("button", { name })
      .first()
      .isVisible()
      .catch(() => false);

  const startButton = await findDeepResearchStartButton(frame);
  const startState = startButton
    ? {
        visible: true,
        enabled: await startButton.isEnabled().catch(() => false),
      }
    : await readDeepResearchStartButtonState(frame);
  const startVisible = startState.visible;
  const startEnabled = startState.enabled;
  const hasPlannerControls =
    (await visibleRole(/^Edit$/i)) || (await visibleRole(/^Cancel$/i));
  const stopResearchVisible =
    (await visibleRole(/^Stop$/i)) || (await visibleRole(/^Stop research$/i));
  const explicitDeepResearchRoot = frame
    .locator(
      "[data-deep-research-root], [data-deep-research-status], [data-deep-research-text]",
    )
    .first();
  const explicitDeepResearchVisible = await explicitDeepResearchRoot
    .isVisible()
    .catch(() => false);
  const isMainFrame = frame === frame.page().mainFrame();
  const mainFrameConversationShell =
    isMainFrame &&
    !(safePageUrl(frame.page())?.toLowerCase() ?? "").includes(
      "/deep-research",
    );
  if (
    mainFrameConversationShell &&
    !startVisible &&
    !stopResearchVisible &&
    !explicitDeepResearchVisible
  ) {
    return null;
  }
  const explicitReportRoot = frame
    .locator(
      "[data-deep-research-text], [class*='reportPage'], article, [role='article'], [role='document'], pre",
    )
    .first();
  const explicitReportVisible = await explicitReportRoot
    .isVisible()
    .catch(() => false);
  const reportRoot = explicitReportVisible
    ? explicitReportRoot
    : frame.locator("main, body").first();
  const statusRoot = frame
    .locator(
      "[data-deep-research-status], [class*='text-token-text-secondary']",
    )
    .first();
  const bodyText = normalize(
    await reportRoot.innerText({ timeout: 100 }).catch(() => ""),
  );
  const statusText = normalize(
    await statusRoot.innerText({ timeout: 100 }).catch(() => ""),
  );
  const lowerUrl = frame.url().toLowerCase();
  const lowerText = `${statusText}\n${bodyText}`.trim().toLowerCase();
  const starting =
    lowerText.includes("starting") ||
    lowerText.includes("getting ready") ||
    lowerText.includes("preparing");
  const generatingReport = lowerText.includes(
    "providing a comprehensive report for the user",
  );
  const plannerShell =
    hasPlannerControls ||
    lowerText.includes("proposed research plan") ||
    lowerText.includes("review the proposed research plan") ||
    lowerText.includes("review the research plan");
  const completed =
    statusText.toLowerCase().includes("research completed") ||
    lowerText.includes("research completed");
  const candidate =
    startVisible ||
    lowerUrl.includes("deep_research") ||
    explicitDeepResearchVisible ||
    stopResearchVisible ||
    lowerText.includes("gathering sources") ||
    lowerText.includes("drafting report") ||
    plannerShell ||
    completed;
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
    !explicitReportVisible &&
    (plannerShell ||
      starting ||
      waiting ||
      generatingReport ||
      stopResearchVisible);
  const reportText = startVisible || provisionalShell ? "" : bodyText;
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

  return normalizeDeepResearchFrameSnapshot({
    phase,
    statusMessage: startVisible
      ? "awaiting start confirmation"
      : statusText || (starting ? "starting research" : lines[0] || null),
    reportText,
    frameUrl: frame.url(),
    startVisible,
    startEnabled,
  });
}
