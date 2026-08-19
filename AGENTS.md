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
