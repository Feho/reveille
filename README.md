<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille

Reveille is a newcomer-first launcher for Medal of Honor: Allied Assault and its two
expansions, Spearhead and Breakthrough. The current Windows v1 identifies an existing
installation, browses servers answering now, checks their map rotations, safely installs exact
missing-map matches, and launches retail MOHAA or OpenMoHAA. On macOS the same overlay installs
the official OpenMoHAA engine **into** a folder that already contains the original `main/`
(and `mainta`/`maintt`) pk3s — it does not replace or delete those files — then launches
`launch_openmohaa_base`, `launch_openmohaa_spearhead`, or `launch_openmohaa_breakthrough`.
The reusable pipeline lives in `reveille-core`; both the CLI proof and Tauri desktop shell call
it directly, while `reveille-platform` holds their shared write-target and process-launch policy.

Each game has its own server list and its own content directory: Allied Assault reads `main`,
Spearhead reads `main` and then `mainta`, Breakthrough reads `main` and then `maintt`. The app
offers only the ones the selected folder actually has, in the toolbar's **Game** switch.

## Run the Windows app

```console
cargo run -p reveille-app
```

That runs the development build. A packaged Windows installer is built by
[`.github/workflows/release.yml`](.github/workflows/release.yml): a `v*` tag produces an NSIS
installer and attaches it to a draft release, and a manual run leaves the same installer as a
workflow artifact. It installs for the current user, so it needs no Administrator, and it fetches
the WebView2 runtime on machines that lack it.

Published releases are also offered inside installed copies of Reveille. The updater uses Tauri's
mandatory release signatures, shows **Update and restart** and **Later**, and never installs from a
background check alone. Before building a release, generate the updater key once:

```console
just updater-key-generate path/to/reveille.key
```

Store the private key as the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`, its password (when
set) as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and the generated public key as the repository
variable `REVEILLE_UPDATER_PUBKEY`. Keep the private key outside the repository and backed up: an
installed release can accept future updates only from that key. To reproduce the signed updater
installer and signature locally, run `just bundle-updater path/to/reveille.key`; leave the signing
prompt empty for a key created without a password.

Prepare a release version without updating Cargo, Tauri and npm independently:

```console
just bump-version 0.1.2
```

The recipe refuses an invalid or non-increasing semantic version and stops if the existing release
identities already disagree. Add `--dry-run` to validate the change without writing it.

**Builds are not code-signed yet.** Windows names no publisher for them and SmartScreen may hold
the download. The signing route is decided — SignPath Foundation's free certificate for open-source
projects — but the application follows the first release rather than preceding it; `docs/plan.md`
records why, and what the certificate does and does not change. winget manifests remain shipping
work outside v1.

## Run on macOS

Reveille does **not** ship the original game. You still need a legal copy of Allied Assault (and
Spearhead / Breakthrough if you play those), with the original pk3s in `main/` (and `mainta` /
`maintt`). Pick that folder in the app. Install OpenMoHAA overlays the official
`openmohaa-v*-macos-multiarch-arm64-x86_64.zip` from
[openmoh/openmohaa releases](https://github.com/openmoh/openmohaa/releases) **on top of** that
folder: engine binaries are added or replaced, game data is left alone. Config and extra maps that
cannot be written into the game folder go to `~/Library/Application Support/openmohaa`, which is
the engine's default home path (`sys_unix.c` under `__APPLE__`). Reveille does not pass
`fs_homepath`.

GitHub Actions still publishes only the Windows NSIS installer (`Reveille_*_x64-setup.exe`). NSIS
does not run on a Mac. Build the app on macOS:

```console
cd crates/reveille-app && npm install
just bundle-macos
```

That is `npx tauri build --bundles app dmg`. It produces `Reveille.app` and a `.dmg` under
`target/release/bundle/`. `cargo run -p reveille-app` is the development build, same as Windows.

**Gatekeeper.** Neither Reveille nor the OpenMoHAA zip is notarized (that needs an Apple Developer
certificate). After a browser or GitHub download, macOS may quarantine the files. If the app or
game will not start, clear quarantine on **your** copies:

```console
xattr -dr com.apple.quarantine /Applications/Reveille.app
xattr -dr com.apple.quarantine /path/to/your/MOHAA
```

That is a local workaround, not notarisation. Do not run it on files you do not trust.

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
