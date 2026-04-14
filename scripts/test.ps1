$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
  $env:PATH = "$cargoBin;$env:PATH"
}

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm lint
pnpm build
pnpm test
