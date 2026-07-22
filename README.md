<div align="center">

# Tablinq

**Use your ChatGPT browser session from local tools.**

Ask questions, send local files, run research, and expose a local gateway while keeping the browser session visible and under your control.

[![Windows first](https://img.shields.io/badge/platform-Windows%20first-2563eb?style=flat-square)](docs/install.md)
[![License](https://img.shields.io/badge/license-Apache--2.0-16a34a?style=flat-square)](LICENSE)

</div>

> **Read this before using Tablinq:** Tablinq is bound to the live ChatGPT website and its browser UI. If that UI changes, Tablinq may break without notice—even while it is running. It is experimental, never intended to be stable, and should be used at your own risk.
>
> Tablinq is developed for Windows first and has not been tested on other operating systems. Contributions are welcome if you want to improve support elsewhere.

## Why use Tablinq?

If you already use ChatGPT in a browser and want local commands or small tools to use that same session, Tablinq gives you a local bridge. You can work from a terminal, pass in files, and keep local jobs and artifacts on your machine without exporting browser cookies or hidden session tokens.

It is useful for:

- turning notes, logs, and documents into concise answers from the terminal;
- asking questions about local files without moving them into another service first;
- running longer research jobs and retrieving their artifacts;
- giving a local script or application an OpenAI-compatible gateway backed by your browser session;
- experimenting with browser-backed workflows where occasional breakage is acceptable.

Tablinq is a good fit for personal, local, and bounded workflows. It is not a stable API replacement, unattended production automation platform, or reliable foundation for autonomous coding agents. See [coding-agent compatibility](docs/coding-agents.md) before attempting that kind of integration.

## Try it on Windows

From a repo checkout, run the installer in PowerShell 7:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/install.ps1
```

Then start an attachable browser session, load the unpacked Tablinq extension into that profile, sign in to ChatGPT manually, and attach Tablinq:

```powershell
gpt browser start-debug
gpt browser attach
gpt doctor
gpt ask "Turn this changelog into three release-note bullets."
```

The complete setup, including browser-profile details and troubleshooting, is in the [installation guide](docs/install.md).

## Explore the project

- [Installation guide](docs/install.md) — set up the local services and attach-first browser flow.
- [CLI and project reference](docs/reference.md) — commands, gateway details, architecture, repository map, and validation.
- [Gateway guide](docs/provider-mode.md) — use the local OpenAI-compatible HTTP surface.
- [Coding-agent compatibility](docs/coding-agents.md) — understand the current support boundary before experimenting.
- [Troubleshooting](docs/troubleshooting.md) — diagnose attach, auth, extension, gateway, and recovery issues.
- [Live smoke checklist](docs/live-smoke-checklist.md) — validate the real ChatGPT browser flow.
- [Contributor guide](docs/development.md) — work from a checkout and run focused validation.

## License

Tablinq is released under the [Apache License 2.0](LICENSE).
