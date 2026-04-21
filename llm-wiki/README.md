# llm-wiki

This directory is a **Karpathy-style LLM Wiki** for the `rust-daq` codebase:
an LLM-maintained, compounding knowledge base written for LLM consumption.

Humans: you can read it, but it is tuned for density and cross-linking, not
for prose. Canonical human docs live under [`docs/`](../docs/) (Diataxis).

- Entry point: [`index.md`](./index.md)
- Conventions / workflows for agents: [`schema.md`](./schema.md)
- Append-only history: [`log.md`](./log.md)

Inspired by Andrej Karpathy's LLM Wiki concept:
<https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f>.

Agents: read `schema.md` before editing anything here. For general work, use
`index.md` for orientation before broad code search, then verify behavior
against source files before making changes. If the source contradicts the wiki,
source wins and the wiki should be corrected.
