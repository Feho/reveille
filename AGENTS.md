<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille repository conventions

- Use default `cargo fmt` formatting. `cargo clippy --workspace --all-targets -- -D warnings`
  must pass.
- Model library errors with `thiserror`. Do not use `unwrap` or `expect` outside tests and the
  executable `main` boundary.
- Keep `reveille-core` policy-free: no terminal output, process spawning, or exit codes. I/O
  presentation and platform policy belong in `reveille-cli` and `reveille-app`.
- Prefer newtypes over bare primitives where mixing values would fail silently, including map
  keys, BSP checksums, client counts, and ports.
- Put a source comment beside every protocol constant (for example, `// sv_gamespy.c:42`) so
  it can be re-verified against the engine.
- Add `SPDX-License-Identifier: GPL-2.0-only` to every new source file. The repository license
  is GPL-2.0-only.

## Where the project context lives

Read these before starting work. They are the whole context — no conversation history is
required, and none survives a machine change.

- [`docs/plan.md`](docs/plan.md) — the v1 plan, milestone by milestone, with a **Status** note on
  each. It records what was measured, what was corrected and why, and the deliberate asymmetries
  that must not be "tidied up" later.
- [`docs/engine-facts.md`](docs/engine-facts.md) — protocol constants, the exact map-name
  normalisation, search-path precedence, frozen fixtures, and prohibitions. Every constant cites
  the openmohaa source line it came from.
- [`docs/ui.md`](docs/ui.md) — the interface: the two decisions it rests on, the honesty rules
  restated as UI rules, the colour tokens with their measured contrast, and the accessibility
  requirements. **Authoritative.** The artifact links below are a convenience and one of them was
  already unreachable to an agent that needed it; everything required to rebuild the interface is
  in this file.
- **PRD** — https://claude.ai/code/artifact/1ffdc89e-6861-4e6b-ab4f-2174d2db2e17
- **Technical blueprint** — https://claude.ai/code/artifact/12731692-491d-4728-9c47-cc741234f839
- **UI mockups** — https://claude.ai/code/artifact/d91bbdfc-9a13-4dea-98d5-39e244d28604 (the
  original study; `docs/ui.md` supersedes it where they differ)

Engine ground truth is the openmohaa source at https://github.com/openmoh/openmohaa. Clone it
when a protocol or filesystem question comes up; every citation in `docs/` points into it.

## Verification habits that produced the current state

- Re-verify a claim against the engine source or a live measurement before building on it.
  Several plan assumptions turned out to be wrong and were caught this way; the corrections are
  recorded in `docs/plan.md` rather than silently applied.
- Never let a network call into a default test. Freeze wire captures and fixtures under
  `crates/reveille-core/tests/fixtures/`; live checks go in separate `#[ignore]` tests.
- An unreachable server, a malformed archive entry, or a failed catalogue lookup is a **recorded
  non-result**, never an aborted pass. The one deliberate exception is `inspect_archive`, which
  hard-rejects a whole downloaded archive — see the note in `docs/plan.md`.
- Report client counts honestly. `numplayers` is `SV_NumClients()`; bots are a separate, disjoint
  quantity. Never merge them, never inflate.
