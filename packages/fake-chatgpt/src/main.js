const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const params = new URLSearchParams(window.location.search);
const variant = params.get("variant") === "aria" ? "aria" : "classic";

document.body.dataset.uiVariant = variant;
note.textContent =
  variant === "aria"
    ? "ARIA-first variant with moved controls and renamed labels."
    : "Classic variant with data-slot anchors and direct toolbar controls.";

root.innerHTML =
  variant === "aria"
    ? `
      <section class="stacked-shell" aria-label="assistant workspace">
        <div class="stacked-toolbar">
          <button type="button" class="pill" aria-haspopup="listbox">
            Choose engine: GPT-5.3 Instant / GPT-5.4 Thinking
          </button>
          <button type="button" class="pill" aria-pressed="false">Browse the web</button>
          <button type="button" class="pill" aria-pressed="false">Deep research</button>
          <button type="button" class="pill" aria-label="Reasoning effort">
            Extended reasoning
          </button>
          <label class="pill" for="upload-input">Add files</label>
          <input id="upload-input" type="file" hidden multiple />
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
        <div class="transcript">
          <article class="message">
            <p>Streaming output lands here.</p>
            <a href="/artifact.txt" download>Download artifact.txt</a>
          </article>
        </div>
      </section>
    `
    : `
      <section class="shell" aria-label="fake-chatgpt-shell">
        <div class="toolbar">
          <button
            type="button"
            data-slot="model-picker"
            aria-haspopup="listbox"
          >
            GPT-5.4 Thinking
          </button>
          <button type="button" data-mode="search">Search</button>
          <button type="button" data-mode="research">Research</button>
          <button type="button" data-slot="effort-toggle">Standard</button>
          <button type="button" data-slot="upload-entry">Attach</button>
        </div>
        <div class="composer">
          <textarea
            aria-label="Prompt composer"
            data-slot="composer"
            placeholder="Ask fake ChatGPT something"
          ></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
        <div class="transcript">
          <article class="message" data-slot="assistant-message">
            <p>Streaming output lands here.</p>
            <a href="/artifact.txt" download data-slot="artifact-link">artifact.txt</a>
          </article>
        </div>
      </section>
    `;

const composer = document.querySelector(
  "[data-slot='composer'], [role='textbox']",
);
const submit = document.querySelector(
  "[data-action='submit'], [aria-label='Submit prompt']",
);
const message =
  document.querySelector("[data-slot='assistant-message'] p") ??
  document.querySelector(".message p");

submit?.addEventListener("click", () => {
  if (!composer || !message) {
    return;
  }

  const prompt =
    "value" in composer ? composer.value.trim() : composer.textContent.trim();

  message.textContent = prompt
    ? `Fake response for: ${prompt}`
    : "Fake response for an empty prompt.";
});
