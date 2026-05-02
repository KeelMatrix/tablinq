# Tablinq Context

Tablinq turns a signed-in ChatGPT browser tab into a local CLI and OpenAI-compatible provider without taking ownership of the user's login session. This file defines durable product and architecture terms that future agents should use consistently.

## Language

**Attach-first**:
The default browser model where Tablinq attaches to a user-owned Chrome or Edge session through a local debug endpoint while the user owns sign-in and the browser process.
_Avoid_: default Playwright login, managed login, headless-first

**Managed fallback**:
A non-default browser path where Tablinq may launch an owned managed browser context when attach mode cannot be enabled or for debugging, CI, or harness isolation.
_Avoid_: default browser mode, normal login path

**Managed only**:
A configured browser mode that skips attach-first and uses the owned managed browser path explicitly.
_Avoid_: attach fallback, normal mode

**Daemon**:
The local `gptd` control process that owns durable execution, persistence, request lifecycle state, and browser-adapter supervision.
_Avoid_: browser adapter, gateway

**Gateway**:
The local OpenAI-compatible HTTP surface that exposes Tablinq through `/v1/*` endpoints backed by the same core service as the daemon path.
_Avoid_: remote API proxy, stateless API server

**Browser adapter**:
The TypeScript Playwright/CDP runtime supervised by Rust to attach to the browser, discover tabs, assist page actions, coordinate files, and report browser state.
_Avoid_: extension, daemon, managed browser

**Extension bridge**:
The MV3 extension path that acts as the semantic bridge between the ChatGPT page and Tablinq's control loop.
_Avoid_: selector pack, Playwright bridge

**Capability probe**:
A page/extension observation that reports available ChatGPT UI capabilities, aliases, and selector-pack identity.
_Avoid_: selector, health check

**Selector pack**:
Versioned JSON selector strategy data under `selector-packs/v1` that is kept outside the compiled browser adapter.
_Avoid_: hardcoded selectors, adapter logic

**State root**:
The local directory tree that stores Tablinq config, logs, SQLite state, downloads, crash bundles, gateway uploads, browser profiles, and daemon lock state.
_Avoid_: project root, install directory

**Failure bundle**:
A persisted diagnostic record for typed failures, usually linking screenshots, DOM or ARIA snapshots, selector attempts, logs, active URL, and capability state.
_Avoid_: log file, crash dump

**Durable job**:
A persisted request/job record whose lifecycle can be inspected after the initiating CLI or gateway client disconnects.
_Avoid_: transient request, provider thread

**Recovery**:
Conservative reconciliation or replay after browser-backed work is interrupted, marked successful only when Tablinq can safely match or rebuild the observed result from durable state.
_Avoid_: blind retry, automatic resend

**Provider page failure**:
A failure surfaced by the ChatGPT page itself, such as `provider_generation_failed_nonretryable`, where Tablinq does not click the page Retry button and uses the documented fresh-conversation recovery path.
_Avoid_: local automation failure, transport failure

**Local automation failure**:
A non-provider-page failure in Tablinq's attach, capability, selector, page-action, upload, download, timeout, or transport control path. Diagnose these failures from **Failure bundle** evidence; do not use the provider page failure replay policy unless the browser page itself reported a **Provider page failure**.
_Avoid_: provider page failure, ChatGPT outage

## Relationships

- A **Gateway** request and a CLI request both route into the same durable control-plane implementation, but not through the same process boundary: the CLI sends daemon requests over local IPC, while the Gateway owns an in-process shared core service and only consults a running Daemon for selected health and doctor status snapshots.
- The **Daemon** supervises one **Browser adapter** process for browser-backed execution.
- The **Browser adapter** attaches to the user's browser in **Attach-first**, or launches owned browser context only in **Managed fallback** or **Managed only**.
- The **Extension bridge** publishes **Capability probe** data that references a **Selector pack**.
- The **State root** contains persisted **Durable job**, **Recovery**, and **Failure bundle** state.
- A **Provider page failure** is classified separately from **Local automation failure** categories such as attach, capability, selector, timeout, upload, download, or transport failures.

## Example dialogue

> **Dev:** "The gateway request failed after the browser tab reloaded; should we resubmit the prompt?"
> **Domain expert:** "No. Treat it as interrupted browser-backed work first: inspect the durable job and recovery metadata, then continue only if Tablinq can safely reconcile or use the documented recovery path."

> **Dev:** "Can I reset the profile to fix Chrome attach-first?"
> **Domain expert:** "No. `reset-profile` belongs to managed fallback recovery; attach-first uses the user-owned attach target, or the dedicated Chrome attach profile under the state root."
