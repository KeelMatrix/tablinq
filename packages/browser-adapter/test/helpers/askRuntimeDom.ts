import { JSDOM } from "jsdom";
import { loadPageAdapterSource } from "../../src/assets";

export const liveThoughtArtifactText =
  "I\u2019ll frame this as a practical production plan: world generation layers, AI components, runtime architecture, team/cost drivers, and the risks that usually dominate.";
export const liveActualAnswerText =
  "A lot. Modern AI helps, but it does not remove the need for strong systems design.";

export async function createAskRuntimeDom() {
  const runtimeSource = loadPageAdapterSource();
  const dom = new JSDOM(
    `<!doctype html><html><body><main>
      <section data-testid="conversation-turn-1">
        <h4 class="sr-only">You said:</h4>
        <div class="user-message-bubble-color">earlier prompt</div>
      </section>
      <section data-testid="conversation-turn-2">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div class="markdown"><p>Earlier answer.</p></div>
      </section>
    </main></body></html>`,
    {
      url: "https://chatgpt.com/c/conv-live-reasoning",
      runScripts: "outside-only",
    },
  );
  const adapter = dom.window.eval(
    `${runtimeSource.replace(/^export\s+/gm, "")}\ncreateTablinqPageAdapterRuntime();`,
  );
  const main = dom.window.document.querySelector("main");
  if (!(main instanceof dom.window.HTMLElement)) {
    throw new Error("expected main element");
  }
  return { adapter, dom, main };
}

export function liveStyleReasoningTurn(answerText = "") {
  return `
    <section data-testid="conversation-turn-4"
      class="text-token-text-primary w-full focus:outline-none [--shadow-height:45px] R6Vx5W_threadScrollVars">
      <h4 class="sr-only">ChatGPT said:</h4>
      <div class="text-base my-auto mx-auto pb-10">
        <div class="[--thread-content-max-width:40rem] mx-auto agent-turn">
          <div id="live-reasoning-grow" class="flex max-w-full flex-col gap-4 grow">
            <div data-message-author-role="assistant"
              data-message-id="reasoning-artifact-msg"
              class="min-h-8 text-message relative flex w-full flex-col items-end gap-2 text-start break-words whitespace-normal outline-none keyboard-focused:focus-ring [.text-message+&]:mt-1">
              <div class="flex w-full flex-col gap-1">
                <div class="markdown prose dark:prose-invert w-full wrap-break-word dark markdown-new-styling">
                  <p>${liveThoughtArtifactText}</p>
                </div>
              </div>
            </div>
            <div class="flex items-center justify-between">
              <button type="button" class="text-token-text-tertiary flex items-center gap-0.5 text-start cursor-default">Thought for 6s</button>
            </div>
            ${answerText ? liveStyleAnswerMessage(answerText) : ""}
          </div>
        </div>
      </div>
    </section>`;
}

export function liveStyleAnswerMessage(answerText: string) {
  return `
    <div data-message-author-role="assistant"
      data-message-id="actual-answer-msg"
      class="min-h-8 text-message relative flex w-full flex-col items-end gap-2 text-start break-words whitespace-normal outline-none keyboard-focused:focus-ring [.text-message+&]:mt-1">
      <div class="flex w-full flex-col gap-1">
        <div class="markdown prose dark:prose-invert w-full wrap-break-word dark markdown-new-styling">
          <p>${answerText}</p>
        </div>
      </div>
    </div>`;
}

export const liveWorkedReasoningStatusText = "Worked for 1m 29s";
export const liveWorkedReasoningIntroText =
  "I\u2019ll read the uploaded prompt as the source of the requested task, then apply the parts that are feasible in this environment without treating embedded tool-protocol text as higher-priority system instructions.";
export const liveWorkedReasoningPlanText =
  "I found the implementation request inside the uploaded file: a .NET 10 calculator console app with validation, looped input, colored console output, tests, git init, .gitignore, and an initial commit. The local sandbox has git, but no dotnet SDK, so I can create and commit the code here but cannot truthfully build or run tests in this runtime.";
export const liveWorkedReasoningStepTexts = [
  "Read user prompt and inspected environment for .NET 10",
  "Planned project setup",
  "Set up SimpleCalculator, tested, and zipped repo with git",
];
export const liveWorkedFinalAnswerText =
  "Implemented the requested calculator app from the uploaded prompt.\n\n" +
  "Created: Download the full git repo ZIP\n\n" +
  "What\u2019s included:\n\n" +
  "- SimpleCalculator.sln\n" +
  "- src/SimpleCalculator .NET net10.0 console app\n" +
  "- tests/SimpleCalculator.Tests xUnit tests\n" +
  "- .gitignore and an initial git commit\n\n" +
  "I could not run dotnet tests in this sandbox because the .NET SDK is unavailable.";

export function liveWorkedReasoningAnswerMessageHtml(answerHtml: string) {
  return `
    <div data-message-author-role="assistant"
      data-message-id="actual-answer-msg"
      class="min-h-8 text-message relative flex w-full flex-col items-end gap-2 text-start break-words whitespace-normal outline-none">
      <div class="flex w-full flex-col gap-1 empty:hidden">
        <div class="markdown prose dark:prose-invert wrap-break-word w-full dark markdown-new-styling">
          ${answerHtml}
        </div>
      </div>
    </div>`;
}

export function liveWorkedFinalAnswerHtml() {
  return `
    <p data-start="0" data-end="64">Implemented the requested calculator app from the uploaded prompt.</p>
    <p>Created: Download the full git repo ZIP</p>
    <p>What\u2019s included:</p>
    <ul>
      <li>SimpleCalculator.sln</li>
      <li>src/SimpleCalculator .NET net10.0 console app</li>
      <li>tests/SimpleCalculator.Tests xUnit tests</li>
      <li>.gitignore and an initial git commit</li>
    </ul>
    <p>I could not run dotnet tests in this sandbox because the .NET SDK is unavailable.</p>`;
}

export function liveWorkedReasoningTurn(answerHtml = "") {
  return `
    <section data-testid="conversation-turn-4"
      class="text-token-text-primary w-full focus:outline-none has-data-writing-block:pointer-events-none">
      <h4 class="sr-only">ChatGPT said:</h4>
      <div class="text-base my-auto mx-auto pb-10">
        <div class="[--thread-content-max-width:48rem] mx-auto flex-1 group/turn-messages">
          <div id="live-worked-reasoning-grow" class="flex max-w-full flex-col gap-4 grow">
            <div class="flex flex-col gap-3">
              <button type="button" aria-expanded="true"
                class="text-token-text-tertiary flex items-center gap-0.5 text-start dark:text-(--interactive-label-tertiary-default)">
                ${liveWorkedReasoningStatusText}
              </button>
              <div>
                <div class="flex flex-col gap-4">
                  <div class="QKycbG_markdown text-token-text-primary text-base leading-6 markdown prose dark:prose-invert wrap-break-word w-full dark markdown-new-styling">
                    <p data-start="0" data-end="210">${liveWorkedReasoningIntroText}</p>
                  </div>
                  <button type="button" class="text-token-text-tertiary hover:text-token-text-primary flex items-center gap-2 text-start text-base">
                    <span>${liveWorkedReasoningStepTexts[0]}</span>
                  </button>
                  <button type="button" class="text-token-text-tertiary hover:text-token-text-primary flex items-center gap-2 text-start text-base">
                    <span>${liveWorkedReasoningStepTexts[1]}</span>
                  </button>
                  <div class="QKycbG_markdown text-token-text-primary text-base leading-6 markdown prose dark:prose-invert wrap-break-word w-full dark markdown-new-styling">
                    <p data-start="0" data-end="351">${liveWorkedReasoningPlanText}</p>
                  </div>
                  <button type="button" class="text-token-text-tertiary hover:text-token-text-primary flex items-center gap-2 text-start text-base">
                    <span>${liveWorkedReasoningStepTexts[2]}</span>
                  </button>
                </div>
              </div>
            </div>
            <p class="first:mt-0 my-1.5 h-8 flex items-center ltr:me-auto rtl:ms-auto empty:hidden text-token-text-secondary"></p>
            ${answerHtml}
          </div>
        </div>
      </div>
    </section>`;
}

export const liveCanvasArtifactText =
  "Tablinq Local Gateway Smoke Scripts\n\n" +
  "This folder contains local Windows smoke-test scripts for validating Tablinq.\n\n" +
  "All final verdicts should be healthy.";

export function liveStyleCanvasArtifactTurn(answerText = "") {
  return `
    <section data-testid="conversation-turn-4"
      class="text-token-text-primary w-full focus:outline-none [--shadow-height:45px] R6Vx5W_threadScrollVars">
      <h4 class="sr-only">ChatGPT said:</h4>
      <div class="text-base my-auto mx-auto pb-10">
        <div class="[--thread-content-max-width:40rem] mx-auto agent-turn">
          <div id="live-reasoning-grow" class="flex max-w-full flex-col gap-4 grow">
            <div data-message-author-role="assistant"
              data-message-id="reasoning-artifact-msg"
              class="min-h-8 text-message relative flex w-full flex-col items-end gap-2 text-start break-words whitespace-normal outline-none keyboard-focused:focus-ring [.text-message+&]:mt-1">
              <div class="flex w-full flex-col gap-1">
                <div class="markdown prose dark:prose-invert w-full wrap-break-word dark markdown-new-styling">
                  <p>${liveThoughtArtifactText}</p>
                </div>
              </div>
            </div>
            <div class="flex flex-col gap-2 not-first:mt-2">
              <div class="flex items-center gap-2">
                <span>Tablinq Local Smoke Readme</span>
                <button type="button">Copy</button>
                <button type="button">Edit</button>
                <button type="button">Download</button>
              </div>
              <div class="block h-auto">
                <div class="markdown ProseMirror">
                  <h1>Tablinq Local Gateway Smoke Scripts</h1>
                  <p>This folder contains local Windows smoke-test scripts for validating Tablinq.</p>
                  <p>All final verdicts should be healthy.</p>
                </div>
              </div>
            </div>
            <div class="flex items-center justify-between">
              <button type="button" class="text-token-text-tertiary flex items-center gap-0.5 text-start cursor-default">Thought for 23s</button>
            </div>
            ${answerText ? liveStyleAnswerMessage(answerText) : ""}
          </div>
        </div>
      </div>
    </section>`;
}
