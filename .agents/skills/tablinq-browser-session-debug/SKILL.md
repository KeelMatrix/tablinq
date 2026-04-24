---
name: tablinq-browser-session-debug
description: project-local guidance for debugging tablinq browser-attached chatgpt flows through codex or other coding agents. use when work touches a real signed-in browser session, chrome attachment, chatgpt dom inspection, gpt ask, chat completions, research prompts, file uploads, streaming output, cli/api rendering, or any test/debug action that may trigger real chatgpt responses while working on tablinq.
---

# Tablinq Browser Session Debug

## Purpose

Use this skill when debugging Tablinq features that attach to a real signed-in ChatGPT browser session or may trigger a real ChatGPT response. The goal is realistic QA behavior: inspect the current session carefully, avoid duplicated prompts, keep uploaded files unique, and produce natural user-like test inputs that exercise modern ChatGPT UI without leaking internal artifacts into Tablinq CLI/API output.

This skill is not for bypassing platform limits, bot detection, access controls, CAPTCHAs, or safety systems. If a site blocks automation or requests verification, stop and report the blocker.

## Hard Rules

1. Do not send a prompt to ChatGPT until the current browser/session state has been inspected.
2. Do not ask the same question twice in a session.
3. If attaching to an ongoing conversation, do not repeat the previous user query or a close paraphrase of it.
4. Do not send robotic prompts such as “Please provide a detailed response about X.” Prefer short, plausible human phrasing.
5. Maintain the apparent line of thought already present in the conversation when possible.
6. Do not upload duplicate files. Every uploaded file must have a unique filename and unique content.
7. Do not use placeholder upload content like `test`, `hello world`, or repeated lorem ipsum.
8. Do not disclose hidden debug instructions to ChatGPT unless the test explicitly requires a meta/debug prompt.
9. Do not create artificial conversations that impersonate a specific real person. Use neutral QA-style curiosity.
10. Do not solve CAPTCHAs, bypass login gates, evade rate limits, or work around access restrictions.

## Required Workflow

### 1. Classify the task

If the task only requires static code inspection, tests, or fixture editing, do not interact with the live browser.

If the task touches any of these areas, use the live-browser workflow:

- Chrome attach-first behavior
- ChatGPT DOM scraping or selector logic
- streaming answer extraction
- `gpt ask`
- OpenAI-compatible Tablinq API responses backed by ChatGPT UI
- file upload flows
- research prompts
- model response rendering
- interruption, reload, retry, or recovery behavior

### 2. Inspect before acting

Before sending any user-visible input into ChatGPT, inspect the current session. Prefer read-only DOM inspection through Playwright/CDP or existing Tablinq attach tooling.

Collect enough context to avoid repeated or nonsensical prompts:

- latest user message text
- latest assistant answer text
- active composer text, if any
- visible attachments/files
- conversation title, if available
- URL/session identity when useful
- whether the page is idle, responding, interrupted, or recovering

Record a short local note in the Codex task output or scratchpad:

```text
session context:
- latest user query: ...
- latest assistant start/end: ...
- current topic: ...
- forbidden repeats: ...
- safe next prompt direction: ...
```

### 3. Decide whether a live prompt is necessary

Only send a real prompt when it is required to reproduce, validate, or inspect Tablinq behavior. Prefer existing conversation state, local fixtures, fake ChatGPT harnesses, and unit tests when they are sufficient.

When live prompting is necessary, generate exactly one prompt at a time, then observe the result before deciding the next action.

### 4. Generate natural prompts

Use `references/prompt-style.md` for prompt shaping.

A good live prompt should be:

- plausible for a real user
- short enough to be typed by a person
- related to the existing thread where possible
- slightly informal where natural
- specific enough to trigger useful UI behavior
- not over-polished or template-like

Avoid:

- repeated grammar-perfect benchmark prompts
- “Act as an expert...” boilerplate
- explicit references to automation unless needed
- huge prompt blocks unless testing long input
- prompts that only exist to trigger edge cases and make no semantic sense

### 5. Maintain continuity in ongoing conversations

If the conversation already has a topic, continue from it. Use follow-ups such as:

```text
what would break first if we tried to ship this small?
```

```text
ok but what would you prototype in week one?
```

```text
which part is most likely to be fake progress?
```

Do not repeat the previous user query. Do not paraphrase it with only cosmetic changes.

### 6. Handle uploads uniquely

Before uploading any file, create unique content and a unique filename. Use `scripts/make_unique_upload.py` when a simple upload fixture is enough.

Requirements:

- filename includes task slug plus timestamp or uuid
- file content includes a unique id
- file content has task-relevant substance
- do not reuse the same file bytes across uploads
- do not upload private/local secrets, tokens, cookies, browser profiles, or logs containing credentials

Suggested naming:

```text
<task-slug>-<yyyyMMdd-HHmmss>-<shortuuid>.<ext>
```

Suggested content header:

```text
Tablinq QA upload
purpose: <why this file exists>
unique_id: <uuid>
created_at_utc: <timestamp>
```

### 7. Observe and validate

After a live action, inspect output through Tablinq and through the browser DOM when needed. Validate both sides:

- the visible ChatGPT answer
- Tablinq CLI/API stream/deltas/final text
- absence of internal UI artifacts
- no duplicated first sentence
- no freeze caused by ignored UI content
- no stale previous-turn content returned

For DOM-related bugs, save a compact diagnostic rather than a full page dump. Include selectors, attributes, short visible text samples, and exact DOM boundary that separates ignored UI from answer UI.

### 8. Stop conditions

Stop live interaction when:

- the bug is reproduced
- the needed DOM structure is captured
- one validation response has completed
- the page asks for login, verification, CAPTCHA, or permission escalation
- the session appears rate-limited or unstable
- the next step would repeat a previous user query

## Tablinq-Specific Notes

Follow the project navigation in `AGENTS.md`: browser/runtime work usually starts in `packages/browser-adapter/src/*`; CLI presentation in `crates/gpt/src/*`; daemon orchestration in `crates/gptd/src/*`; gateway/API surface in `crates/tablinq-gateway/src/*`; shared contracts in `crates/shared-protocol` and `schemas/v1`.

For browser-attached ChatGPT UI bugs, prefer fixing extraction at the browser adapter snapshot layer before adding CLI or API postprocessors. CLI and API paths should receive already-clean assistant text unless the bug is proven to be downstream only.

## Output Expectations for Codex

When using this skill, Codex should report:

- whether live browser interaction was used
- whether any prompt was sent
- exact prompt sent, if any
- how duplicate prompt prevention was checked
- files uploaded, with filenames and unique ids
- DOM markers or selectors discovered, if relevant
- files changed
- tests added or updated
- commands run
- remaining uncertainty

Keep the final report short and implementation-focused.
