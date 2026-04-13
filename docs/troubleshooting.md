# Troubleshooting

## Browser not detected

- Run `gpt doctor` and inspect `browser_detection`.
- On Windows, verify `msedge.exe` or `chrome.exe` exists in a standard install path.
- If needed, set `browser_path` in `%LOCALAPPDATA%\Tablinq\config\config.toml`.

## Not authenticated or wrong account

- Run `gpt status` or `gpt doctor` and check `auth.authenticated`.
- If the managed profile is signed out or using the wrong account, run:
  - `gpt browser logout`
  - or `gpt browser switch-account`
- If the profile is corrupted or stuck, run `gpt browser reset-profile`.

## Native messaging does not connect

- Re-run `scripts/install.ps1` to refresh registry keys and native-host manifests.
- Confirm the unpacked extension path matches `%LOCALAPPDATA%\Tablinq\install\extension`.
- Check `%LOCALAPPDATA%\Tablinq\logs\native-host.log` if present.

## Browser adapter fails after install

- Run `gpt version --json` and confirm the package was built from the expected commit.
- Verify `%LOCALAPPDATA%\Tablinq\install\browser-adapter\src\service.js` exists.
- The daemon prefers packaged `service.js` output and falls back to the repo `service.ts` entry only in a developer checkout.

## Gateway is unhealthy

- Run `gpt gateway status`.
- Check `GET /health`, `GET /ready`, and `GET /v1/provider`.
- Inspect `%LOCALAPPDATA%\Tablinq\logs\gateway.log`.
- Restart in order:
  - `gpt gateway stop`
  - `gpt daemon stop`
  - `gpt daemon start`
  - `gpt gateway start`

## Reset local state

- `gpt browser reset-profile` recreates the managed browser profile.
- Delete `%LOCALAPPDATA%\Tablinq\downloads` to remove managed artifacts.
- Delete `%LOCALAPPDATA%\Tablinq\crash-bundles` to clear saved failure bundles.
- Delete `%LOCALAPPDATA%\Tablinq\gateway-uploads` to clear staged provider uploads.

## Selector-pack updates

- Edit `selector-packs/v1/chatgpt-default.json` before packaging, or replace the installed copy under `%LOCALAPPDATA%\Tablinq\install\selector-packs`.
- Restart `gptd` after changing selector-pack data so the next probe reloads it.
