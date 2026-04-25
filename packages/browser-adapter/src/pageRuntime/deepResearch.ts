import type { Frame, Page } from "playwright-core";

import type {
  DeepResearchFrameSnapshot,
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
  deepResearchStartDelayMs,
} from "../timing";
import { safePageUrl } from "../capabilityProbe";

import { callPageAdapter } from "./adapter";
import { clickWithInteractionJitter, sleep } from "./common";
import { readResearchSubmissionState } from "./researchSubmission";

export async function beginDeepResearchRun(
  page: Page,
  timeoutMs: number,
): Promise<ResearchSnapshot | null> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frameSnapshot = await readDeepResearchFrameSnapshot(page);
    if (frameSnapshot?.startVisible || frameSnapshot?.phase === "planning") {
      if (frameSnapshot.startVisible && !frameSnapshot.startEnabled) {
        await sleep(
          Math.min(
            adaptivePollIntervalMs("edge", Date.now() - startedAt),
            Math.max(1, deadline - Date.now()),
          ),
        );
        continue;
      }
      await sleep(
        Math.min(
          deepResearchStartDelayMs(),
          Math.max(0, deadline - Date.now()),
        ),
      );
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

    const submission = await readResearchSubmissionState(page).catch(
      () => null,
    );
    if (
      submission &&
      !submission.onDeepResearchLanding &&
      submission.status !== "idle"
    ) {
      return researchSubmissionStateToSnapshot(submission);
    }

    const merged = await readLiveResearchSnapshot(page).catch(() => null);
    if (
      merged &&
      merged.status !== "idle" &&
      merged.statusMessage !== "awaiting start confirmation"
    ) {
      return merged;
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
    const button = frame.getByRole("button", { name: /^Start$/i }).first();
    const visible = await button.isVisible().catch(() => false);
    if (!visible) {
      continue;
    }
    const enabled = await button.isEnabled().catch(() => false);
    if (!enabled) {
      continue;
    }
    const clicked = await clickWithInteractionJitter(button, "button_click", {
      force: true,
      timeout: INTERACTION_TIMEOUT_MS,
    })
      .then(() => true)
      .catch(() => false);
    if (clicked) {
      return true;
    }
  }
  return false;
}

export function deepResearchCandidateFrames(page: Page): Frame[] {
  return page.frames().filter((frame) => frame !== page.mainFrame());
}

export async function readDeepResearchFrameSnapshot(
  page: Page,
): Promise<DeepResearchFrameSnapshot | null> {
  const snapshots: DeepResearchFrameSnapshot[] = [];
  for (const frame of deepResearchCandidateFrames(page)) {
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

  const startButton = frame.getByRole("button", { name: /^Start$/i }).first();
  const startVisible = await startButton.isVisible().catch(() => false);
  const startEnabled = startVisible
    ? await startButton.isEnabled().catch(() => false)
    : false;
  const hasPlannerControls =
    (await visibleRole(/^Edit$/i)) || (await visibleRole(/^Cancel$/i));
  const stopResearchVisible =
    (await visibleRole(/^Stop$/i)) || (await visibleRole(/^Stop research$/i));
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
  const candidate =
    startVisible ||
    lowerUrl.includes("deep_research") ||
    lowerUrl.includes("oaiusercontent") ||
    lowerText.includes("deep research") ||
    stopResearchVisible ||
    lowerText.includes("gathering sources") ||
    lowerText.includes("drafting report") ||
    lowerText.includes("edit") ||
    lowerText.includes("cancel");
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
  const completed =
    statusText.toLowerCase().includes("research completed") ||
    lowerText.includes("research completed");
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
