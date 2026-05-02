import { deepResearchPathname } from "./scenarioState.js";

export function renderHarnessShell({ root, note, window, config }) {
  const {
    variant,
    scenario,
    usesComposerPlusTrap,
    usesStaleSubmitRecovery,
    usesDeepResearchFrame,
    usesConversationLoadFailure,
    missingControls,
    effortInConfigureModal,
  } = config;

  note.textContent =
    variant === "aria"
      ? "ARIA-first variant with moved controls, renamed labels, and semantic groups."
      : variant === "composer-right"
        ? "Composer-right variant with the model picker beside the composer and nested thinking effort controls."
        : variant === "menu"
          ? "Current menu variant with a ChatGPT trigger, portal menus, and composer-adjacent effort controls."
          : variant === "live"
            ? usesComposerPlusTrap
              ? "Current live variant with a ChatGPT trigger, portal menus, and a composer plus button trap."
              : "Current live variant with a ChatGPT trigger, portal menus, and composer-adjacent effort controls."
            : "Classic variant with grouped controls and direct toolbar buttons.";

  function hasControl(name) {
    return !missingControls.has(name);
  }

  function classicModelControls() {
    if (!hasControl("model")) {
      return "";
    }
    return `
      <div class="pill" data-slot="model-picker" role="group" aria-label="Model picker">
        <button type="button" data-model="instant" aria-pressed="false">GPT-5.3 Instant</button>
        <button type="button" data-model="thinking" aria-pressed="true">GPT-5.4 Thinking</button>
      </div>
    `;
  }

  function classicModeControls() {
    return [
      hasControl("search")
        ? `<button type="button" data-mode="search" aria-pressed="false">Search</button>`
        : "",
      hasControl("research")
        ? `<button type="button" data-mode="research" aria-pressed="false">Research</button>`
        : "",
    ].join("");
  }

  function classicEffortControls() {
    if (!hasControl("effort")) {
      return "";
    }
    return `
      <div class="pill" data-slot="effort-toggle" role="group" aria-label="Thinking effort">
        <button type="button" data-effort="standard" aria-pressed="true">Standard</button>
        <button type="button" data-effort="extended" aria-pressed="false">Extended</button>
      </div>
    `;
  }

  function classicUploadControl() {
    if (!hasControl("upload")) {
      return "";
    }
    return `
      <button type="button" data-slot="upload-entry">Attach</button>
      <input id="classic-upload-input" type="file" hidden multiple />
    `;
  }

  function ariaModelControls() {
    if (!hasControl("model")) {
      return "";
    }
    return `
      <div class="pill" role="group" aria-label="Choose engine">
        <button type="button" data-model="instant" aria-pressed="false">Instant answers</button>
        <button type="button" data-model="thinking" aria-pressed="true">Deep thinking</button>
      </div>
    `;
  }

  function ariaModeControls() {
    return [
      hasControl("search")
        ? `<button type="button" data-mode="search" aria-pressed="false">Search</button>`
        : "",
      hasControl("research")
        ? `<button type="button" data-mode="research" aria-pressed="false">Deep research</button>`
        : "",
    ].join("");
  }

  function ariaEffortControls() {
    if (!hasControl("effort")) {
      return "";
    }
    return `
      <div class="pill" role="group" aria-label="Reasoning effort">
        <button type="button" data-effort="standard" aria-pressed="true">Standard reasoning</button>
        <button type="button" data-effort="extended" aria-pressed="false">Extended reasoning</button>
      </div>
    `;
  }

  function ariaUploadControl() {
    if (!hasControl("upload")) {
      return "";
    }
    return `
      <label class="pill" for="upload-input">Add files</label>
      <input id="upload-input" type="file" hidden multiple />
    `;
  }

  function liveModelControls() {
    if (!hasControl("model")) {
      return "";
    }
    return `
      <div class="pill">
        <button
          type="button"
          id="model-switcher-trigger"
          data-testid="model-switcher-dropdown-button"
          aria-label="Model selector"
          aria-haspopup="menu"
          aria-controls="tablinq-model-menu"
          aria-expanded="false"
        >
          ChatGPT
        </button>
      </div>
    `;
  }

  function menuModelControls() {
    return liveModelControls();
  }

  function liveModeControls() {
    return "";
  }

  function liveSidebarNav() {
    if (
      variant !== "live" &&
      variant !== "menu" &&
      variant !== "composer-right"
    ) {
      return "";
    }
    return `
      <nav class="stacked-toolbar" aria-label="workspace navigation">
        <a href="/" data-testid="create-new-chat-button">New chat</a>
        <button type="button" class="group __menu-item hoverable w-full">Search chats</button>
        <a href="/apps" data-testid="apps-button">Apps</a>
        ${
          hasControl("research")
            ? `<a href="${deepResearchPathname(window.location.pathname)}${window.location.search}" data-testid="deep-research-sidebar-item" id="deep-research-nav">Deep research</a>`
            : ""
        }
        <a href="/codex/cloud">Codex</a>
        <a href="/gpts" data-testid="explore-gpts-button">GPTs</a>
        <button type="button" id="projects-nav-button">Projects</button>
      </nav>
    `;
  }

  function conversationLoadFailureBanner() {
    if (!usesConversationLoadFailure) {
      return "";
    }
    const pathSegments = window.location.pathname.split("/").filter(Boolean);
    const conversationId = pathSegments[pathSegments.length - 1] ?? "unknown";
    return `
      <div role="alert" class="bg-token-bg-danger">
        Unable to load conversation ${conversationId}
      </div>
    `;
  }

  function liveEffortControls() {
    if (!hasControl("effort")) {
      return "";
    }
    return `
      <button
        type="button"
        id="thinking-effort-trigger"
        class="pill __composer-pill"
        aria-label="Thinking"
        aria-haspopup="menu"
        aria-controls="tablinq-effort-menu"
        aria-expanded="false"
        hidden
      >
        Thinking
      </button>
    `;
  }

  function menuEffortControls() {
    return liveEffortControls();
  }

  function composerRightModelTrigger() {
    if (!hasControl("model")) {
      return "";
    }
    return `
      <button
        type="button"
        class="pill __composer-pill"
        data-harness-model-trigger="composer-right"
        aria-haspopup="menu"
        aria-controls="tablinq-model-menu"
        aria-expanded="false"
      >
        Instant
      </button>
    `;
  }

  function currentPortalMenus() {
    if (
      variant !== "live" &&
      variant !== "menu" &&
      variant !== "composer-right"
    ) {
      return "";
    }
    return `
      <div data-slot="portal-root">
        ${
          hasControl("model")
            ? `
        <div id="tablinq-model-menu" role="menu" hidden>
          <div class="pill" aria-hidden="true">Latest</div>
          ${
            hasControl("instant-model")
              ? `
          <div id="model-option-instant" role="menuitemradio" data-testid="${variant === "composer-right" ? "model-switcher-gpt-5-3-instant" : "model-switcher-gpt-5-3"}" hidden>
            <div>Instant</div>
            <div>For everyday chats</div>
          </div>
        `
              : ""
          }
          ${
            hasControl("thinking-model")
              ? `
          <div id="model-option-thinking" role="menuitemradio" data-testid="model-switcher-gpt-5-4-thinking" hidden>
            <div>Thinking</div>
            <div>For complex questions</div>
            ${
              variant === "composer-right"
                ? effortInConfigureModal
                  ? ""
                  : `<button type="button" role="menuitem" data-testid="model-switcher-gpt-5-4-thinking-thinking-effort" aria-label="Effort" aria-haspopup="menu" aria-controls="tablinq-effort-menu" aria-expanded="false"></button>`
                : ""
            }
          </div>
        `
              : ""
          }
          <div id="model-option-pro" role="menuitemradio" data-testid="model-switcher-gpt-5-4-pro" hidden>
            <div>Pro</div>
            <div>Research-grade intelligence</div>
          </div>
          ${
            hasControl("effort") && effortInConfigureModal
              ? `
          <div id="model-option-configure" role="menuitem" data-testid="model-configure-modal" hidden>
            Configure...
          </div>
          `
              : ""
          }
        </div>
        `
            : ""
        }
        ${
          hasControl("effort")
            ? `
        <div id="tablinq-effort-menu" role="menu" hidden>
          <div class="pill" aria-hidden="true">Thinking effort</div>
          <div id="effort-option-standard" role="menuitemradio" hidden>Standard</div>
          <div id="effort-option-extended" role="menuitemradio" hidden>Extended</div>
        </div>
        `
            : ""
        }
        ${
          hasControl("effort") && effortInConfigureModal
            ? `
        <div id="tablinq-intelligence-modal" data-testid="modal-intelligence-menu" hidden>
          <section role="dialog" aria-label="Intelligence">
            <h2>Intelligence</h2>
            <button type="button" role="radio" id="modal-model-instant" aria-checked="false">Instant 5.3 For everyday chats</button>
            <button type="button" role="radio" id="modal-model-thinking" aria-checked="false">Thinking 5.5 For complex questions</button>
            <button type="button" role="radio" id="modal-model-pro" aria-checked="false">Pro 5.5 Research-grade intelligence</button>
            <label for="modal-effort-combobox">Thinking effort</label>
            <button type="button" id="modal-effort-combobox" role="combobox" aria-expanded="false">Standard</button>
            <div id="modal-effort-listbox" role="listbox" hidden>
              <div id="modal-effort-standard" role="option" aria-selected="false" hidden>Standard</div>
              <div id="modal-effort-extended" role="option" aria-selected="false" hidden>Extended</div>
            </div>
          </section>
        </div>
        `
            : ""
        }
        ${
          hasControl("search")
            ? `
        <div id="tablinq-tools-menu" role="menu" hidden>
          <div id="tool-option-web-search" role="menuitemradio" hidden>Web search</div>
        </div>
        `
            : ""
        }
      </div>
    `;
  }

  function liveUploadControl() {
    if (!hasControl("upload")) {
      return "";
    }
    if (usesComposerPlusTrap) {
      return `
        <input id="live-upload-input" type="file" hidden multiple />
      `;
    }
    return `
      <button type="button" data-testid="upload-button" aria-label="Add photos and files">Add files</button>
      <input id="live-upload-input" type="file" hidden multiple />
    `;
  }

  function liveComposerActions() {
    if (usesComposerPlusTrap) {
      return `
        <div class="pill">
          ${liveEffortControls()}
          <button
            type="button"
            id="composer-plus-btn"
            data-testid="composer-plus-btn"
            aria-label="Add files and more"
            aria-haspopup="menu"
            aria-controls="tablinq-tools-menu"
            aria-expanded="false"
          >
            +
          </button>
          <button type="button" id="composer-send-glyph">
            <span aria-hidden="true">></span>
          </button>
          <button
            type="button"
            data-testid="stop-button"
            aria-label="Stop generating"
            hidden
          >
            Stop
          </button>
        </div>
      `;
    }
    return `
      <div class="pill">
        ${variant === "composer-right" ? composerRightModelTrigger() : liveEffortControls()}
        ${
          hasControl("search")
            ? `<button type="button" id="search-chip" class="__composer-pill" aria-label="Search" hidden>Search</button>`
            : ""
        }
        ${
          hasControl("search")
            ? `
        <button
          type="button"
          id="composer-plus-btn"
          data-testid="composer-plus-btn"
          aria-label="Add files and more"
          aria-haspopup="menu"
          aria-controls="tablinq-tools-menu"
          aria-expanded="false"
        >
          +
        </button>
        `
            : ""
        }
        <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        <button
          type="button"
          data-testid="stop-button"
          aria-label="Stop generating"
          hidden
        >
          Stop
        </button>
      </div>
    `;
  }

  function liveDecoySubmitControl() {
    if (!usesStaleSubmitRecovery) {
      return "";
    }
    return `
      <section class="stacked-toolbar" aria-label="decoy controls">
        <button
          type="button"
          id="stale-send-button"
          data-testid="send-button"
          aria-label="Send prompt"
          hidden
        >
          Send
        </button>
      </section>
    `;
  }

  root.innerHTML =
    variant === "aria"
      ? `
        <section class="stacked-shell" aria-label="assistant workspace">
          <div class="stacked-toolbar">
            ${ariaModelControls()}
            ${ariaModeControls()}
            ${ariaEffortControls()}
            ${ariaUploadControl()}
          </div>
          <div class="stacked-composer">
            <div
              role="textbox"
              aria-label="Message ChatGPT"
              contenteditable="true"
              spellcheck="false"
            ></div>
            <button type="button" aria-label="Submit prompt">Go</button>
          </div>
          <div class="stacked-toolbar" data-slot="upload-list"></div>
          <div class="transcript">
            <article
              class="message"
              data-slot="assistant-message"
              data-stream-state="idle"
            >
              <p>Streaming output lands here.</p>
              <a
                href="/artifact.txt"
                download="artifact.txt"
                data-slot="artifact-link"
                data-artifact-ready="false"
              >
                Download artifact.txt
              </a>
            </article>
          </div>
        </section>
      `
      : variant === "live"
        ? `
        <main aria-label="chatgpt workspace">
          ${liveSidebarNav()}
          ${conversationLoadFailureBanner()}
          <section class="stacked-toolbar">
            ${liveModelControls()}
            ${liveModeControls()}
            ${liveUploadControl()}
          </section>
          ${liveDecoySubmitControl()}
          <form data-testid="composer-root" class="stacked-composer">
            <textarea
              id="prompt-textarea"
              aria-label="Message ChatGPT"
              placeholder="Message ChatGPT"
            ></textarea>
            ${liveComposerActions()}
          </form>
          <div class="stacked-toolbar" data-slot="upload-list"></div>
          <section class="transcript">
            <article
              class="message"
              data-message-author-role="assistant"
              data-stream-state="idle"
            >
              <div class="markdown">
                <p>Streaming output lands here.</p>
              </div>
              ${
                scenario === "followup-rich-text" ||
                scenario === "recover-rich-markdown"
                  ? ""
                  : `
              <a
                href="#"
                data-testid="artifact-download"
                aria-label="Download report"
              >
                Download report
              </a>
              `
              }
            </article>
          </section>
          <section class="notice-panel" aria-label="workspace notices">
            <article class="message" data-slot="notice-card">
              <p>Workspace notices appear here while ChatGPT is thinking.</p>
            </article>
          </section>
          ${
            usesDeepResearchFrame
              ? `
          <iframe
            id="deep-research-sandbox"
            title="internal://deep-research"
          ></iframe>
          `
              : ""
          }
          ${currentPortalMenus()}
        </main>
      `
        : variant === "menu" || variant === "composer-right"
          ? `
        <main aria-label="chatgpt workspace">
          ${liveSidebarNav()}
          ${conversationLoadFailureBanner()}
          <section class="stacked-toolbar">
            ${variant === "composer-right" ? "" : menuModelControls()}
            ${liveModeControls()}
            ${liveUploadControl()}
          </section>
          <form data-testid="composer-root" class="stacked-composer">
            <textarea
              id="prompt-textarea"
              aria-label="Message ChatGPT"
              placeholder="Message ChatGPT"
            ></textarea>
            ${liveComposerActions()}
          </form>
          <div class="stacked-toolbar" data-slot="upload-list"></div>
          <section class="transcript">
            <article
              class="message"
              data-message-author-role="assistant"
              data-stream-state="idle"
            >
              <div class="markdown">
                <p>Streaming output lands here.</p>
              </div>
              <a
                href="#"
                data-testid="artifact-download"
                aria-label="Download report"
              >
                Download report
              </a>
            </article>
          </section>
          ${currentPortalMenus()}
        </main>
      `
          : `
        <section class="shell" aria-label="fake-chatgpt-shell">
          <div class="toolbar">
            ${classicModelControls()}
            ${classicModeControls()}
            ${classicEffortControls()}
            ${classicUploadControl()}
          </div>
          <div class="composer">
            <textarea
              aria-label="Prompt composer"
              data-slot="composer"
              placeholder="Ask fake ChatGPT something"
            ></textarea>
            <button type="button" data-action="submit">Send</button>
          </div>
          <div class="toolbar" data-slot="upload-list"></div>
          <div class="transcript">
            <article
              class="message"
              data-slot="assistant-message"
              data-stream-state="idle"
            >
              <p>Streaming output lands here.</p>
              <a
                href="/artifact.txt"
                download="artifact.txt"
                data-slot="artifact-link"
                data-artifact-ready="false"
              >
                artifact.txt
              </a>
            </article>
          </div>
        </section>
      `;
}
