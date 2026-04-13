# Install

## Windows 10-11

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Build a local package with `scripts/package.ps1`.
4. Install it with `scripts/install.ps1`.
5. Load the unpacked extension from `%LOCALAPPDATA%\\Tablinq\\install\\extension` in Edge or Chrome.
6. Start the daemon with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe daemon start`.
7. Open the managed browser with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe browser open`.
8. Log in once in the managed profile, then use `ask`, `research`, `doctor`, and the artifact commands normally.

`scripts/install.ps1` creates the state directories under `%LOCALAPPDATA%\\Tablinq`, registers the user-level native messaging host, checks for Edge or Chrome, and prints the installed extension path plus the stable extension ID.

## Unix-like systems

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Build a local package with `./scripts/package.sh`.
4. Install it with `./scripts/install.sh`.
5. Load the unpacked extension from `~/.local/share/tablinq/install/extension`.

The Unix install script is intentionally lighter than the Windows path. It prepares the install and state directories and verifies the CLI can bootstrap config, but native-host registration is still Windows-first.
