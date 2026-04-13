# Troubleshooting

## Browser not detected

- Run `gpt doctor` and check `browser_detection`.
- On Windows, verify `msedge.exe` or `chrome.exe` exists in the standard install paths.
- If needed, set `browser_path` in `%LOCALAPPDATA%\\Tablinq\\config\\config.toml`.

## Native messaging does not connect

- Re-run `scripts/install.ps1` to refresh the registry keys and native-host manifests.
- Confirm the unpacked extension path matches `%LOCALAPPDATA%\\Tablinq\\install\\extension`.
- Check `%LOCALAPPDATA%\\Tablinq\\logs\\native-host.log` for incoming host messages.

## Browser adapter fails after install

- Run `gpt version --json` and confirm the package was built from the expected commit.
- Verify `%LOCALAPPDATA%\\Tablinq\\install\\browser-adapter\\src\\service.js` exists.
- `gptd` prefers the packaged `service.js` entry when present and falls back to the repo `service.ts` entry in a developer checkout.

## Reset local state

- Run `gpt browser reset-profile` to recreate the managed browser profile.
- Delete `%LOCALAPPDATA%\\Tablinq\\downloads` if you want to remove managed artifacts.
- Delete `%LOCALAPPDATA%\\Tablinq\\crash-bundles` if you want to clear saved failure bundles.

## Selector-pack updates

- Edit `selector-packs/v1/chatgpt-default.json` in the repo before packaging, or replace the installed copy under `%LOCALAPPDATA%\\Tablinq\\install\\selector-packs`.
- Restart `gptd` after changing selector-pack data so the next probe run reloads it.
