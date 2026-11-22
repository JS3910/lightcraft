# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

This repo is **single-context**, and upstream keeps neither a `GLOSSARY.md` nor ADRs. Its vocabulary and decisions live in its own docs, so read those instead:

- **`CLAUDE.md`** (project rules) and the docs it points to: `ROADMAP.md`, `docs/gaps.md`, `docs/parity-checklist.md`, `docs/target-app-parity.md`.
- **The feature doc for the area you touch**, for example `docs/faces.md` for People and faces (face, unnamed face, suggestion, confirm, person), `docs/control-protocol.md`, `docs/background-tasks.md`.
- **`GLOSSARY.md`** at the repo root and **`docs/adr/`**, should they ever exist.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The owner has decided not to add a `GLOSSARY.md` here: this is not their codebase, and its existing terminology is kept. Record changed behaviour in the project's own docs, following its conventions.

## File structure

```
/
├── CLAUDE.md, ROADMAP.md
├── docs/            ← feature docs, parity tracker, gaps (the vocabulary source)
└── crates/, apps/
```

## Use the project's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term the project's docs and code already use. Don't drift to synonyms, and don't rename existing concepts.

If the concept you need has no existing term, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (name it plainly and say it is new).

## Flag conflicts with documented decisions

If your output contradicts a rule in `CLAUDE.md` or a decision recorded in a feature doc, surface it explicitly rather than silently overriding it.
