# Live Smoke Checklist

This checklist is optional and stays out of CI. Use it only when you want to validate the managed browser flow against the real ChatGPT site with your local profile.

## Preconditions

- Install locally with `scripts/install.ps1` or `./scripts/install.sh`.
- Start the daemon with `gpt daemon start`.
- Open the managed browser with `gpt browser open`.
- Log in through the managed profile if required.
- Confirm `gpt doctor` reports a healthy daemon, detected browser, extension connectivity, and a non-empty capability summary.

## Smoke checks

1. Basic ask: run `gpt ask "hello from tablinq"` and confirm streamed output appears without hanging.
2. Instant model: run `gpt ask --model instant "summarize this in one sentence"` and confirm the request completes or fails cleanly with `capability missing`.
3. Thinking standard: run `gpt ask --model thinking --effort standard "compare rust and typescript for local tooling"` and confirm completion or a clean capability error.
4. Thinking extended: run `gpt ask --model thinking --effort extended "reason carefully about this tradeoff"` and confirm completion or a clean capability error.
5. Search mode: run `gpt ask --mode search "latest rust stable release"` and confirm the request completes with the requested mode or fails cleanly if the control is unavailable.
6. Upload: run `gpt ask --file path\\to\\notes.txt "summarize the attached file"` and confirm the page acknowledges the attachment before submission.
7. Download: after a run that produces an artifact, confirm `gpt artifacts list` shows it and `gpt artifacts pull last` retrieves it successfully.
8. Research: run `gpt research "investigate launch blockers"` and confirm progress states advance through the local job view until completion or a typed failure.
9. Recovery: after at least one download, confirm the managed browser tab is still usable by running `gpt capabilities` or another `gpt ask`.

## What to capture on failure

- `gpt doctor`
- `gpt capabilities`
- the failing CLI command
- the latest bundle under the managed `crash-bundles/` directory
- a note describing the visible UI drift or missing control
