import {
  accessibleProbeName,
  createProbeNeighborhood,
  firstVisibleProbeElement,
  implicitProbeRole,
  isVisibleProbeElement,
  normalizeProbeText,
  probeTextHasToken,
} from "./text.js";
import { controlCandidateSelector } from "./strategies.js";

export function createProbeContext(documentRoot = document) {
  const normalizeText = normalizeProbeText;
  const isVisible = isVisibleProbeElement;
  const firstVisible = (elements) =>
    firstVisibleProbeElement(elements, isVisible);
  const implicitRole = implicitProbeRole;
  const accessibleName = (element) =>
    accessibleProbeName(element, documentRoot);
  const hasToken = probeTextHasToken;

  const firstVisibleBySelector = (selector) =>
    firstVisible(documentRoot.querySelectorAll(selector));

  const firstVisibleBySelectors = (selectors) => {
    for (const selector of selectors) {
      const match = firstVisibleBySelector(selector);
      if (match) {
        return match;
      }
    }
    return null;
  };

  const findNamedControl = (tokens, options = {}) => {
    const { include, includeHidden = false, roots = [documentRoot] } = options;
    for (const root of roots) {
      const candidates =
        root instanceof Document || root instanceof Element
          ? root.querySelectorAll(controlCandidateSelector)
          : [];
      for (const candidate of Array.from(candidates)) {
        if (!includeHidden && !isVisible(candidate)) {
          continue;
        }
        if (include && !include(candidate)) {
          continue;
        }
        const name = accessibleName(candidate);
        if (hasToken(name, tokens)) {
          return candidate;
        }
      }
    }
    return null;
  };

  const findTextbox = () => {
    const candidates = documentRoot.querySelectorAll(
      [
        "#prompt-textarea",
        "[data-slot='composer']",
        "[data-testid='composer-root'] textarea",
        "[data-testid='composer-root'] [contenteditable='true']",
        "textarea[aria-label*='Message']",
        "textarea[placeholder*='message']",
        "[role='textbox'][aria-label*='Message']",
        "[contenteditable='true'][aria-label*='Message']",
        "textarea",
        "[role='textbox']",
        "[contenteditable='true']",
      ].join(", "),
    );
    for (const candidate of Array.from(candidates)) {
      if (!isVisible(candidate)) {
        continue;
      }
      const name = accessibleName(candidate);
      if (
        candidate.getAttribute("data-slot") === "composer" ||
        hasToken(name, ["prompt", "message", "composer", "ask", "chat"])
      ) {
        return candidate;
      }
      if (
        candidate.tagName.toLowerCase() === "textarea" ||
        candidate.getAttribute("role") === "textbox" ||
        candidate.isContentEditable
      ) {
        return candidate;
      }
    }
    return null;
  };

  const composerContainers = () => {
    const composer = findTextbox();
    const seen = new Set();
    const containers = [];
    for (const container of [
      composer?.closest("[data-testid='composer-root']"),
      composer?.closest("form"),
      composer?.parentElement,
      composer?.closest("section"),
      composer?.closest("main"),
    ]) {
      if (!(container instanceof HTMLElement) || seen.has(container)) {
        continue;
      }
      seen.add(container);
      containers.push(container);
    }
    return containers;
  };

  const isComposerAdjacent = (element) =>
    element instanceof HTMLElement &&
    composerContainers().some(
      (container) => container === element || container.contains(element),
    );

  const composerAdjacencyScore = (element) => {
    const composer = findTextbox();
    if (
      !(composer instanceof HTMLElement) ||
      !(element instanceof HTMLElement)
    ) {
      return 0;
    }
    let score = 0;
    if (element.closest("form") === composer.closest("form")) {
      score += 6;
    }
    if (element.parentElement === composer.parentElement) {
      score += 4;
    }
    if (element.closest("section") === composer.closest("section")) {
      score += 2;
    }
    if (element.closest("main") === composer.closest("main")) {
      score += 1;
    }
    return score;
  };

  const findComposerAdjacentNamedControl = (tokens, options = {}) =>
    findNamedControl(tokens, {
      ...options,
      roots: composerContainers(),
    });

  const createNeighborhood = (capability, strategyId, element) =>
    createProbeNeighborhood(capability, strategyId, element);

  const safeQueryAll = (root, selector) => {
    try {
      return Array.from(root.querySelectorAll(selector));
    } catch {
      return [];
    }
  };

  const genericControlSelector = [
    controlCandidateSelector,
    "textarea",
    "[role='textbox']",
    "[contenteditable='true']",
    "input[type='file']",
    "input[type='submit']",
    "a[download]",
  ].join(", ");

  const visibleGenericControls = (roots = [documentRoot]) => {
    const seen = new Set();
    const controls = [];
    for (const root of roots) {
      for (const element of safeQueryAll(root, genericControlSelector)) {
        if (!isVisible(element) || seen.has(element)) {
          continue;
        }
        seen.add(element);
        controls.push(element);
      }
    }
    return controls;
  };

  return {
    documentRoot,
    body: documentRoot.body,
    normalizeText,
    isVisible,
    firstVisible,
    implicitRole,
    accessibleName,
    hasToken,
    controlCandidateSelector,
    firstVisibleBySelector,
    firstVisibleBySelectors,
    findNamedControl,
    findTextbox,
    composerContainers,
    isComposerAdjacent,
    composerAdjacencyScore,
    findComposerAdjacentNamedControl,
    createNeighborhood,
    safeQueryAll,
    visibleGenericControls,
  };
}
