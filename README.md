<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille

Reveille is a newcomer-first launcher for Medal of Honor: Allied Assault. The current
implementation is the headless launcher foundation: it identifies existing asset trees,
indexes maps using the engine's search-path precedence, preflights server rotations, resolves
missing content, and constructs launch commands without executing them.

## Scan an install

```console
cargo run -p reveille-cli -- scan /path/to/MOHAA
```

The scan reports the number of archives and maps, duplicate map providers, and the effective
checksum for every map. Providers are ordered in engine lookup order; the first provider is the
file the engine loads.

## Browse public servers

```console
cargo run -p reveille-cli -- browse --path /path/to/MOHAA
```

Use `--limit N` for a smaller sample, `--game spearhead` or `--game breakthrough` for an
expansion, and `--format json` for the complete structured report and per-server compatibility
assessments. Displayed client counts are the server's non-free slots; Reveille never labels them
players or humans.

## Prepare a join

```console
cargo run -p reveille-cli -- join 203.0.113.10:12203 /path/to/MOHAA
```

This runs status discovery, rotation preflight, and content-source resolution, then prints a
typed launch command. It does not install content or start a process.

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```
