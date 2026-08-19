<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille

Reveille is a newcomer-first launcher for Medal of Honor: Allied Assault. The current
implementation is the headless content foundation: it identifies an existing asset tree,
indexes maps using the engine's search-path precedence, and preflights server rotations.

## Scan an install

```console
cargo run -p reveille-cli -- scan /path/to/MOHAA
```

The scan reports the number of archives and maps, duplicate map providers, and the effective
checksum for every map. Providers are ordered in engine lookup order; the first provider is the
file the engine loads.

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```
