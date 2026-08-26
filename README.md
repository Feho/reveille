<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille

Reveille is a newcomer-first launcher for Medal of Honor: Allied Assault and its two
expansions, Spearhead and Breakthrough. The current Windows v1 identifies an existing
installation, browses servers answering now, checks their map rotations, safely installs exact
missing-map matches, and launches retail MOHAA or OpenMoHAA. The reusable pipeline lives in
`reveille-core`; both the CLI proof and Tauri desktop shell call it directly, while
`reveille-platform` holds their shared Windows write-target and process-launch policy.

Each game has its own server list and its own content directory: Allied Assault reads `main`,
Spearhead reads `main` and then `mainta`, Breakthrough reads `main` and then `maintt`. The app
offers only the ones the selected folder actually has, in the toolbar's **Game** switch.

## Run the Windows app

```console
cargo run -p reveille-app
```

The development build is intentionally unpackaged and unsigned. Packaging, winget manifests,
and Microsoft Store submission are shipping work outside v1.

## Prove Journey B in one command

```console
cargo run -p reveille-cli -- journey 203.0.113.10:12203 --path "C:\Games\MOHAA" --execute
```

This detects and identifies the install, performs a complete live browse, preflights the chosen
server, resolves and installs safe exact map matches, rescans, and launches only when the final
check is Compatible. Omit `--execute` to leave the launch step as a printed command.

## Scan an install

```console
cargo run -p reveille-cli -- scan /path/to/MOHAA
```

The scan reports the number of archives and maps, duplicate map providers, and the effective
checksum for every map. Providers are ordered in engine lookup order; the first provider is the
file the engine loads. Add `--game spearhead` or `--game breakthrough` to index that game's
search path instead, which is the expansion's directory over `main` rather than in place of it.

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
