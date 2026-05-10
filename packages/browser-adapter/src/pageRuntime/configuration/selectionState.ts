import type { Page } from "playwright-core";

export type VisibleComposerSelectionState = {
  texts: string[];
  hasVisibleModelState: boolean;
  hasUnsupportedPro: boolean;
  model: "instant" | "thinking" | "none" | "ambiguous";
  effort: "standard" | "extended" | "none" | "ambiguous";
};

type ConfiguredVisibleSelectionRequest = {
  modelClass?: "instant" | "thinking" | null;
  effort?: "none" | "standard" | "extended" | null;
};

export async function readVisibleComposerSelectionState(
  page: Page,
): Promise<VisibleComposerSelectionState> {
  return page
    .evaluate<VisibleComposerSelectionState>(
      `(() => {
      const normalize = (value) =>
        (value ?? "").replace(/\\s+/g, " ").trim().toLowerCase();
      const isVisible = (element) => {
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
      const texts = Array.from(
        document.querySelectorAll(
          "#thinking-effort-trigger, button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
        ),
      )
        .filter((element) => isVisible(element))
        .flatMap((element) => [
          normalize(element.textContent),
          normalize(element.getAttribute("aria-label")),
        ])
        .filter(Boolean)
        .filter((value, index, values) => values.indexOf(value) === index);
      const hasUnsupportedPro = texts.some((text) => text.includes("pro"));
      const hasInstant = texts.some(
        (text) => text.includes("instant") || text.includes("5.3"),
      );
      const hasSupportedThinking = texts.some(
        (text) =>
          !text.includes("pro") &&
          (text.includes("thinking") || text.includes("5.4")),
      );
      const hasAmbiguousFiveFive = texts.some(
        (text) =>
          text.includes("5.5") &&
          !text.includes("pro") &&
          !text.includes("instant") &&
          !text.includes("thinking") &&
          !text.includes("standard") &&
          !text.includes("extended") &&
          !text.includes("5.3") &&
          !text.includes("5.4"),
      );
      let model = "none";
      if (hasUnsupportedPro || hasAmbiguousFiveFive || (hasInstant && hasSupportedThinking)) {
        model = "ambiguous";
      } else if (hasInstant) {
        model = "instant";
      } else if (hasSupportedThinking) {
        model = "thinking";
      }
      let effort = "none";
      if (hasUnsupportedPro || hasAmbiguousFiveFive) {
        effort = "ambiguous";
      } else if (model === "thinking") {
        effort = texts.some((text) => text.includes("extended"))
          ? "extended"
          : "standard";
      }
      return {
        texts,
        hasVisibleModelState: texts.length > 0,
        hasUnsupportedPro,
        model,
        effort,
      };
    })()`,
    )
    .catch(() => ({
      texts: [],
      hasVisibleModelState: false,
      hasUnsupportedPro: false,
      model: "none",
      effort: "none",
    }));
}

async function readSelectedComposerControlState(
  page: Page,
): Promise<VisibleComposerSelectionState> {
  return page
    .evaluate<VisibleComposerSelectionState>(
      `(() => {
      const normalize = (value) =>
        (value ?? "").replace(/\\s+/g, " ").trim().toLowerCase();
      const texts = Array.from(
        document.querySelectorAll(
          [
            "[aria-pressed='true']",
            "[aria-checked='true']",
            "[data-selected='true']",
            "[data-state='checked']",
          ].join(", "),
        ),
      )
        .flatMap((element) => [
          normalize(element.textContent),
          normalize(element.getAttribute("aria-label")),
          normalize(element.getAttribute("data-model")),
          normalize(element.getAttribute("data-effort")),
        ])
        .filter(Boolean)
        .filter((value, index, values) => values.indexOf(value) === index);
      const hasUnsupportedPro = texts.some((text) => text.includes("pro"));
      const hasInstant = texts.some(
        (text) => text.includes("instant") || text.includes("5.3"),
      );
      const hasSupportedThinking = texts.some(
        (text) =>
          !text.includes("pro") &&
          (text.includes("thinking") || text.includes("5.4")),
      );
      let model = "none";
      if (hasUnsupportedPro || (hasInstant && hasSupportedThinking)) {
        model = "ambiguous";
      } else if (hasInstant) {
        model = "instant";
      } else if (hasSupportedThinking) {
        model = "thinking";
      }
      let effort = "none";
      if (hasUnsupportedPro) {
        effort = "ambiguous";
      } else if (texts.some((text) => text.includes("extended"))) {
        effort = "extended";
      } else if (
        texts.some((text) => text.includes("standard")) ||
        model === "thinking"
      ) {
        effort = "standard";
      }
      return {
        texts,
        hasVisibleModelState: texts.length > 0,
        hasUnsupportedPro,
        model,
        effort,
      };
    })()`,
    )
    .catch(() => ({
      texts: [],
      hasVisibleModelState: false,
      hasUnsupportedPro: false,
      model: "none",
      effort: "none",
    }));
}

export function visibleSelectionMatchesModelClass(
  state: VisibleComposerSelectionState,
  modelClass: "instant" | "thinking",
): boolean {
  if (state.hasUnsupportedPro) {
    return false;
  }
  if (modelClass === "instant") {
    if (state.model === "instant") {
      return true;
    }
    if (state.model === "thinking" || state.model === "ambiguous") {
      return false;
    }
    return !state.hasVisibleModelState;
  }
  return state.model === "thinking";
}

export function visibleSelectionMatchesEffort(
  state: VisibleComposerSelectionState,
  effort: "none" | "standard" | "extended",
): boolean {
  if (effort === "none") {
    return true;
  }
  if (state.hasUnsupportedPro || state.model !== "thinking") {
    return false;
  }
  return state.effort === effort;
}

export function describeVisibleComposerSelection(
  state: VisibleComposerSelectionState,
): string {
  return state.texts.length > 0 ? state.texts.join(" | ") : "none";
}

export async function assertVisibleComposerSelection(
  page: Page,
  request: ConfiguredVisibleSelectionRequest,
): Promise<void> {
  if (!request.modelClass && (!request.effort || request.effort === "none")) {
    return;
  }

  const state = await readVisibleComposerSelectionState(page);
  const requestedModel = request.modelClass ?? "unspecified";
  const requestedEffort = request.effort ?? "none";
  const visible = describeVisibleComposerSelection(state);
  const selectedState = await readSelectedComposerControlState(page);
  const visibleEffort: VisibleComposerSelectionState["effort"] =
    state.texts.some((text) => text.includes("extended"))
      ? "extended"
      : state.texts.some((text) => text.includes("standard"))
        ? "standard"
        : "none";
  const effectiveState =
    state.hasVisibleModelState &&
    state.model === "none" &&
    request.modelClass === "thinking" &&
    visibleEffort !== "none" &&
    !selectedState.hasUnsupportedPro
      ? { ...state, model: "thinking" as const, effort: visibleEffort }
      : state.hasVisibleModelState
        ? state
        : selectedState;

  if (
    state.hasUnsupportedPro ||
    selectedState.hasUnsupportedPro ||
    effectiveState.hasUnsupportedPro
  ) {
    throw new Error(
      `capability missing: unsupported Pro model state is selected; requested_model=${requestedModel}; requested_effort=${requestedEffort}; visible=${visible}`,
    );
  }

  if (
    request.modelClass &&
    !visibleSelectionMatchesModelClass(effectiveState, request.modelClass)
  ) {
    throw new Error(
      `ui drift: visible composer model state does not satisfy requested model=${request.modelClass}; visible=${visible}; selected=${describeVisibleComposerSelection(effectiveState)}`,
    );
  }

  if (
    request.effort &&
    request.effort !== "none" &&
    !visibleSelectionMatchesEffort(effectiveState, request.effort)
  ) {
    throw new Error(
      `ui drift: visible composer effort state does not satisfy requested effort=${request.effort}; visible=${visible}; selected=${describeVisibleComposerSelection(effectiveState)}`,
    );
  }
}
