# Domain docs

This repo uses a single-context domain layout.

Expected files:

- `CONTEXT.md` at the repo root for shared domain language
- `docs/adr/` for architectural decision records

Skills should read `CONTEXT.md` before changing code in unfamiliar areas. Use `docs/adr/` to understand prior architectural decisions and avoid re-litigating settled tradeoffs.

If `CONTEXT.md` does not exist yet, create it lazily when `/grill-with-docs` resolves the first durable domain term.
If `docs/adr/` does not exist yet, create it lazily when a decision is hard to reverse, surprising without context, and based on a real tradeoff.
