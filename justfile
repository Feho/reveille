# SPDX-License-Identifier: GPL-2.0-only

# Recipes are single commands that read the same under `sh` and under `cmd`, so the same justfile
# serves the Windows machine and the aarch64 Linux one. Two things forced that:
#
#   * `just` runs recipes under `sh` on every platform unless told otherwise, and Windows has no
#     `sh`. Hence `windows-shell` below.
#   * `bash` on a Windows PATH is normally the WSL shim in System32, not Git Bash — a
#     different filesystem with a different toolchain. Pointing at it would be worse than `cmd`.
#
# `cmd` rather than PowerShell because Windows PowerShell 5.1 has neither `&&` nor `||`, which
# `bundle` and `engine-source` rely on.
set windows-shell := ["cmd.exe", "/c"]

# Where to keep the openmohaa checkout used as engine ground truth. Outside the repo by default
# so it never lands in `git status`.
engine_source := "../openmohaa"

# List the recipes.
default:
    @just --list

# ---------------------------------------------------------------------------
# Gates. `just check` is what CI runs; run it before pushing.
# ---------------------------------------------------------------------------

# Everything CI checks, in the order that fails cheapest first.
check: fmt-check lint test sources

# Apply the canonical formatting.
fmt:
    cargo fmt --all

# Fail if anything is unformatted.
fmt-check:
    cargo fmt --all --check

# The workspace lint gate. `-D warnings` is not negotiable; see CLAUDE.md.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# The whole workspace, offline.
test:
    cargo test --workspace --locked

# Nothing else catches either of these: the frontend has no build step, so a syntax error there
# first shows up as a blank window, and no compiler enforces the licence header CLAUDE.md
# requires on every source file.

# Check SPDX headers and that the shell's JavaScript parses.
sources:
    node tools/check-sources.mjs

# ---------------------------------------------------------------------------
# Portability. `reveille-core`, `reveille-cli`, and `reveille-platform` must keep building off
# Windows — that is what makes the macOS OpenMoHAA overlay (and a later Linux shell) deferred
# rather than precluded (docs/plan.md, "Cross-platform posture"). Running these on Windows will
# not prove a non-Windows target, but it does catch an accidental `winreg` dependency in core.
# ---------------------------------------------------------------------------

# The ubuntu CI leg.
portable: portable-test portable-lint fmt-check

# Test only the crates that must build off Windows.
portable-test:
    cargo test -p reveille-core -p reveille-cli -p reveille-platform --locked

# Lint only the crates that must build off Windows.
portable-lint:
    cargo clippy -p reveille-core -p reveille-cli -p reveille-platform --all-targets --locked -- -D warnings

# ---------------------------------------------------------------------------
# Live checks. Never part of `just check`: a network call must never reach a default test
# (CLAUDE.md). Each of these talks to a third party and can fail for reasons that are not bugs.
# ---------------------------------------------------------------------------

# Every #[ignore]d test at once.
live:
    cargo test --workspace --locked -- --ignored --nocapture

# Does the latest release still publish a digest-bearing archive for this host? Run it after any
# change to the asset selector or to the frozen release fixture.

# Live check against the official GitHub Releases API.
live-release:
    cargo test -p reveille-core --test live_openmohaa_release --locked -- --ignored --nocapture

# The GameSpy master and public UDP servers.
live-discovery:
    cargo test -p reveille-core --test live_discovery --locked -- --ignored --nocapture

# The third-party moh-db catalogue.
live-catalogue:
    cargo test -p reveille-core --test live_catalogue --locked -- --ignored --nocapture

# ---------------------------------------------------------------------------
# Running the thing.
# ---------------------------------------------------------------------------

# `frontendDist` is a static directory, so there is no dev server to start and no frontend build
# step: edit `crates/reveille-app/ui` and re-run.

# Run the Tauri shell.
app:
    cargo run -p reveille-app

# The shell as players get it. Slow to build; use it to check real timing and window behaviour.
app-release:
    cargo run -p reveille-app --release

# Produce the Windows NSIS installer. Needs the npm dev dependency: `cd crates/reveille-app && npm install`.
# NSIS does not run on macOS; use `just bundle-macos` there.
bundle:
    cd crates/reveille-app && npx -- tauri build --bundles nsis

# Produce Reveille.app and a .dmg. Must be run on macOS; this is not an NSIS installer.
bundle-macos:
    cd crates/reveille-app && npx -- tauri build --bundles app dmg

# Generate the updater key once; an empty password is valid, and the private key needs backup.
updater-key-generate KEY:
    cd crates/reveille-app && npm run tauri signer generate -- -w "{{ KEY }}"

# Build signed updater artifacts from KEY and its `.pub` sibling; set the password env var if used.
bundle-updater KEY:
    node tools/build-updater.mjs "{{ KEY }}"

# Update Cargo, Tauri and npm release identities together; VERSION must be newer SemVer.
bump-version VERSION *ARGS:
    node tools/bump-version.mjs "{{ VERSION }}" {{ ARGS }}

# The headless pipeline. `just cli --help` lists the subcommands.
cli *ARGS:
    cargo run -p reveille-cli -- {{ ARGS }}

# Installations from the live Windows registry.
discover:
    cargo run -p reveille-cli -- discover

# Identify an install and index its maps. Add `--game spearhead` or `--game breakthrough` for an
# expansion's search path, which is its own directory over `main` rather than in place of it.
scan PATH *ARGS:
    cargo run -p reveille-cli -- scan "{{ PATH }}" {{ ARGS }}

# The whole journey against one server, stopping short of launching it. Add `--execute` to launch.
journey SERVER *ARGS:
    cargo run -p reveille-cli -- journey {{ SERVER }} {{ ARGS }}

# ---------------------------------------------------------------------------
# Engine ground truth. Every protocol constant in docs/engine-facts.md cites a line in this
# source; re-verify against it rather than against a previous claim (CLAUDE.md).
# ---------------------------------------------------------------------------

# Clone openmohaa, or fast-forward an existing checkout.
engine-source:
    git clone --depth 1 https://github.com/openmoh/openmohaa "{{ engine_source }}" || git -C "{{ engine_source }}" pull --ff-only

# Find where a constant or function actually comes from, with file:line to cite.
engine-grep PATTERN:
    git -C "{{ engine_source }}" grep -n -- "{{ PATTERN }}"
