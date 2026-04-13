# Install

## Windows 10-11

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Build a local package with `scripts/package.ps1`.
4. Install it with `scripts/install.ps1`.
   - Optional autostart:
     - `scripts/install.ps1 -AutoStartDaemon`
     - `scripts/install.ps1 -AutoStartGateway`
     - `scripts/install.ps1 -AutoStartDaemon -AutoStartGateway`
5. Load the unpacked extension from `%LOCALAPPDATA%\\Tablinq\\install\\extension` in Edge or Chrome.
6. Start the daemon with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe daemon start`.
7. Start the gateway with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe gateway start`.
8. Open the managed browser with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe browser open`.
9. Log in once in the managed profile.
10. Check the merged operational view with `%LOCALAPPDATA%\\Tablinq\\install\\bin\\gpt.exe status`.

`scripts/install.ps1` creates the state directories under `%LOCALAPPDATA%\\Tablinq`, registers the user-level native messaging host, installs the CLI, daemon, gateway, browser-adapter assets, and extension bundle, checks for Edge or Chrome, and writes startup helper scripts under `%LOCALAPPDATA%\\Tablinq\\install\\startup`.

## Unix-like systems

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Build a local package with `./scripts/package.sh`.
4. Install it with `./scripts/install.sh`.
5. Load the unpacked extension from `~/.local/share/tablinq/install/extension`.
6. Start the daemon with `~/.local/share/tablinq/install/bin/gpt daemon start`.
7. Start the gateway with `~/.local/share/tablinq/install/bin/gpt gateway start`.
8. Check the merged operational view with `~/.local/share/tablinq/install/bin/gpt status`.

The Unix install script is intentionally lighter than the Windows path. It prepares the install and state directories, verifies the CLI can bootstrap config, and writes startup helper scripts under the installed `startup` directory, but native-host registration is still Windows-first.
