export const DEEP_RESEARCH_START_BUTTON_NAME = /^Start(?:\s+\d+)?$/i;
export const DEEP_RESEARCH_START_BUTTON_TEXT = /^Start\b/i;
export const DEEP_RESEARCH_START_BUTTON_SELECTOR = "button, [role='button']";

export interface DeepResearchStartButtonState {
  visible: boolean;
  enabled: boolean;
}

export interface DeepResearchStartDomResult extends DeepResearchStartButtonState {
  clicked: boolean;
}

export interface DeepResearchStartDomOptions {
  action: "classify" | "read" | "click";
  requirePlanContext?: boolean;
  requireNestedRoot?: boolean;
  rootSelector?: string;
  composerControlSelector: string;
  composerSurfaceSelector: string;
}

export const DEEP_RESEARCH_START_DOM_OPTIONS = {
  composerControlSelector: [
    "[data-testid='composer-root']",
    "[data-testid='composer-footer-actions']",
    "[data-testid='composer-action-file-upload']",
    "[data-testid='send-button']",
    "#prompt-textarea",
    "textarea",
    "[role='textbox']",
  ].join(","),
  composerSurfaceSelector:
    "[data-testid='composer-root'], #prompt-textarea, textarea, [role='textbox']",
};

export function evaluateDeepResearchStartDom(
  targetOrOptions: Element | DeepResearchStartDomOptions,
  maybeOptions?: DeepResearchStartDomOptions,
): DeepResearchStartDomResult {
  const options = maybeOptions ??
    (targetOrOptions as DeepResearchStartDomOptions | undefined) ?? {
      action: "read",
      composerControlSelector: "",
      composerSurfaceSelector: "",
    };
  const root =
    maybeOptions && targetOrOptions instanceof Element
      ? targetOrOptions
      : document;
  const requirePlanContext = options.requirePlanContext !== false;
  const normalize = (value: string | null | undefined) =>
    String(value ?? "")
      .replace(/\u00a0/g, " ")
      .replace(/\s+/g, " ")
      .trim();
  const documents = (): Document[] => {
    const rootDocument =
      root instanceof Document ? root : (root.ownerDocument ?? document);
    const nestedFrame = rootDocument.querySelector(
      options.rootSelector ?? "iframe#root",
    );
    const nestedDocument =
      nestedFrame instanceof HTMLIFrameElement
        ? nestedFrame.contentDocument
        : null;
    if (options.requireNestedRoot) {
      return nestedDocument ? [nestedDocument] : [];
    }
    return nestedDocument ? [rootDocument, nestedDocument] : [rootDocument];
  };
  const isVisible = (element: Element): element is HTMLElement => {
    if (!(element instanceof HTMLElement) || element.hidden) {
      return false;
    }
    const style = element.ownerDocument.defaultView?.getComputedStyle(element);
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
  const hasPlanText = (value: string) => {
    const lower = value.toLowerCase();
    return (
      (lower.includes("edit") && lower.includes("cancel")) ||
      lower.includes("research plan") ||
      lower.includes("proposed research")
    );
  };
  const isComposerControl = (element: HTMLElement): boolean => {
    if (
      options.composerControlSelector &&
      element.closest(options.composerControlSelector)
    ) {
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
      options.composerSurfaceSelector &&
      form.querySelector(options.composerSurfaceSelector) instanceof
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
  const isDisabled = (element: HTMLElement): boolean => {
    const nativeDisabled =
      "disabled" in element &&
      Boolean((element as HTMLButtonElement | HTMLInputElement).disabled);
    return (
      nativeDisabled ||
      element.hasAttribute("disabled") ||
      element.getAttribute("aria-disabled") === "true"
    );
  };
  const isStartButton = (element: Element): element is HTMLElement => {
    if (!isVisible(element) || isComposerControl(element)) {
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
    if (!requirePlanContext) {
      return true;
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
  const stateFor = (button: HTMLElement | null): DeepResearchStartDomResult => {
    if (!(button instanceof HTMLElement)) {
      return { visible: false, enabled: false, clicked: false };
    }
    return {
      visible: true,
      enabled: !isDisabled(button),
      clicked: false,
    };
  };

  if (options.action === "classify") {
    return stateFor(
      root instanceof HTMLElement && isStartButton(root) ? root : null,
    );
  }

  let button: HTMLElement | undefined;
  for (const rootDocument of documents()) {
    button = Array.from(
      rootDocument.querySelectorAll("button, [role='button']"),
    ).find(isStartButton);
    if (button) {
      break;
    }
  }
  const state = stateFor(button ?? null);
  if (options.action !== "click" || !button || !state.enabled) {
    return state;
  }
  button.click();
  return { ...state, clicked: true };
}
