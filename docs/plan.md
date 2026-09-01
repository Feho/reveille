<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille v1 — full implementation plan

## Context

Reveille is a newcomer-first launcher for Medal of Honor: Allied Assault: install the game,
keep it current, browse servers that actually answer, and fetch the maps a join needs before
the join fails. Its PRD and technical blueprint are written, independently reviewed, corrected
against the engine source, and published. Milestones 1-5 Part A are implemented; see the per-milestone **Status** notes below.

The reviewed success criterion is **ten minutes from valid game assets on disk to being in a
game with other people**, for someone who has never played MOHAA and cannot ask for help.
Everything below is sequenced to protect that.

**Licence GPL-2.0**, matching openmohaa. New git repo at `/home/feho/dev/reveille`.

## Which machine — recommendation

**Stay here for milestones 1–4; move to Windows for 5–6.** Not a compromise: it follows from
what each machine can actually verify.

This machine (aarch64 Linux) uniquely has the three things the protocol and content work must
be checked against — the **openmohaa engine source** as ground truth, a **live dedicated
server** to query, and the **real retail pk3 corpus** (Pak0–Pak7Fr plus 20 custom archives).
Roughly three quarters of v1 by code volume is headless Rust that this environment tests better
than Windows would.

It cannot do the rest, and the gaps are not incidental: Windows install discovery (registry,
GOG Galaxy manifests, EA App), the Tauri webview shell, launching the game,
SmartScreen and signing, and Journey A end to end — which *is* the success
criterion. `aarch64` also means any Tauri build here targets the wrong platform anyway.

**The honest consequence:** install detection is the PRD's first v1 bullet but is Windows-specific,
so it moves later in the sequence than the PRD's ordering implies. Milestone 1 handles the
platform-neutral half (*given a path, what is this install?*) and milestone 5 handles the
Windows-specific half (*where are the installs?*).

Windows CI (`windows-latest`) runs from the first commit so the target never rots.

## Fixture reality

`/home/feho/MOHAA` is a **dedicated server**, not a client — `MOHAA_server.exe`, `omohaaded`,
`game.so`; no `mohaa.exe` or client binary anywhere on this machine. That splits fixture
coverage cleanly:

- **Valid here:** the map index, BSP checksums, preflight. `main/` holds the genuine retail
  pk3 set plus real custom archives, identical to what a client would load.
- **Not valid here:** install *discovery* and binary fingerprinting. Those get synthetic
  `tempfile` fixtures in milestone 1 and real verification in milestone 5.
- Also present: `main/` has only `main` (no `mainta`/`maintt`), a Spearhead pk3 and a
  Breakthrough SFX pk3 sitting inside an AA server, and a filename with a space
  (`z_Kmarzo-St Renan.pk3`). Useful mess — the indexer should survive all of it.

## Verified before planning

- **Map checksum is the third `int32` of the BSP header**, little-endian, offset 8.
  `CM_Checksum` (`code/qcommon/cm_load.c:752-771`) has its lump-hashing path commented out and
  returns `header->checksum`; `CM_LoadMap` uses it at `cm_load.c:887`. Confirmed live: local
  `Pak5.pk3` yields `dm/mohdm6 → 1974169620` and `dm/mohdm7 → 391868696`, exactly the
  `sv_mapChecksum` three independent servers reported. **A 12-byte read, not a BSP parser.**
- BSP header is `ident = "2015"` on Allied Assault maps and **`EALA`** on expansion maps
  repackaged for AA servers (70 and 18 of the corpus's 88, respectively); `version = 19`
  throughout the corpus. **Gate on version, not ident** — `CM_LoadMap` (`cm_load.c:894`) never
  inspects `ident` and errors out when `version` falls outside `BSP_MIN_VERSION` 17 ..
  `BSP_MAX_VERSION` 21 (`qfiles.h:361-364`). The checksum offset is fixed regardless.
- **Search-path precedence** (`code/qcommon/files.cpp:3106-3240`): `fs_searchpaths` is a
  prepend-to-head list walked from the head, so **last added wins**. Within a game directory,
  pk3s and `.pk3dir`s are added alphabetically (later alphabetically wins — what `zzzz_`
  prefixes exploit) and **the plain game directory is added last**, so loose files beat every
  pk3. Lookups are case- and separator-insensitive (`FS_FilenameCompare`, `files.cpp:1362`).
- Out-of-band header is five bytes, not Quake 3's four: `ff ff ff ff 02` out, `ff ff ff ff 01`
  in. The plain Q3 header gets silence, indistinguishable from a closed port.
- `hostport` from the GameSpy reply is the authoritative game port and is **not** a fixed
  offset (12203 typical, 23900 observed).
- Snapshot, 19 Aug 2026 09:44: 169 registered, 130 reachable, 127 answering `getstatus`,
  **127/127 on protocol 8**, 104 retail, 88 clients reported, `sv_maplist` on 98/127,
  `sv_mapChecksum` on 30/127, `pr_downloads` on 1/127, `pure` on **0/127**.

## Structure

```
/home/feho/dev/reveille/
├── Cargo.toml            workspace
├── LICENSE               GPL-2.0
├── .github/workflows/    test on ubuntu + windows-latest from commit 1
└── crates/
    ├── reveille-core/    all logic — no GUI, no platform policy
    ├── reveille-cli/     headless driver; the full pipeline runnable here
    └── reveille-app/     Tauri shell (milestone 6, Windows)
```

Dependencies kept small and boring: `zip`, `sha2`, `md-5`, `walkdir`, `serde`/`serde_json`,
`thiserror`, `tokio` + `reqwest` (rustls) for network, `clap` for the CLI.

---

## Milestone 1 — content foundations *(here)*

Replaces `/home/feho/MOHAA/tools/spike_fingerprint.py`.

- **`bsp`** — read `ident` and the offset-8 checksum from a `.bsp` header without inflating the
  whole entry.
- **`mapindex`** — walk `.pk3` archives **and** loose `maps/**/*.bsp` **and** `.pk3dir`
  directories, modelling the precedence above. Key on a case-folded, separator-normalised name
  (`maps/dm/mohdm6.bsp` → `dm/mohdm6`) while preserving original spelling for display — one
  live `sv_maplist` contained both `DM/mohdm6` and `dm/mohdm7`. Return **all** providers ordered
  by engine load order with each provider's checksum. **Scan the game directory only, not
  recursively** — `FS_AddGameDirectory` (`files.cpp:3136`) lists `*.pk3` with `wantSubs =
  qfalse`, so `main/disabled_mods/` and any nested copies are invisible to the engine and must
  be invisible here too.
- **`install::identify(path)`** — the platform-neutral half: recognised binaries hashed with
  SHA-256, expansions by data directory (`main`→`mohaa`, `mainta`→`mohaas`, `maintt`→`mohaab`).
  The hash→version corpus starts **empty** and grows from known-good installs, so identification
  must expose *how* it decided as an enum, never a bare version string. A live server reports
  `sv_info "MoH:AA 1.12 Reborn Patch RC3.5"` while its `gamever` says `1.11` — this is real.
- **`preflight`** — given a rotation and an optional `sv_mapChecksum`, report per map:
  present / present-but-checksum-differs / absent. Returns a structured verdict, never a bool:
  the documents commit to "compatible" meaning *nothing we can check is wrong*, and a bool
  erases that at the type level.

**Verify:** `scan /home/feho/MOHAA` → 30 pk3s, **88 maps**, **0 multi-provider**;
`dm/mohdm6 → 1974169620` from `Pak5.pk3`; `dm/mohdm7 → 391868696`. Loose-file precedence,
`.pk3dir` precedence, pk3-vs-pk3 alphabetical precedence and case-folding get synthetic
`tempfile` fixtures — with an engine-scoped scan the real install has no map shadowing at all,
so the synthetic tests are precedence's *only* coverage.

**Status: complete** (commit `b1ea641`). 15 tests, clippy and fmt clean. The earlier
`54 multi-provider` acceptance figure was wrong — it came from a recursive scan that swept up
`.claude/worktrees/*/main` and `main/disabled_mods` copies of the same archives. The engine
loads none of those; the corrected figure is 0.

## Milestone 2 — discovery *(here)*

Ports `spike_masterlist.py` and `spike_pipeline.py`.

- GameSpy v1 master client: TCP 28900, challenge → `gs_encrypt` (RC4 variant) → `gs_encode`
  (zero-padded base64, no `=`) → validate. Keys from `code/gamespy/sv_gamespy.c`.
- UDP GameSpy `\status\` query; read **`hostport`** for the game port — never compute it.
- MOHAA out-of-band `getstatus` / `getinfo` on the game port with the five-byte header.
- Bounded concurrency, per-host timeouts, reachable-only filtering.
- Server model carries **clients reported** (never "players" — `numplayers` is
  `SV_NumClients()`, every non-free slot), version, protocol, rotation, `sv_allowDownload`,
  `sv_mapChecksum`, `pr_downloads`, ping band, join window, reserved slots.

**Verify:** against the live server here and the full master list; reproduce the 19 Aug figures
within drift, and assert `hostport` is read rather than assumed.

**Status: implemented** (commits `34f41d7`, `1660f1c`). 26 tests pass, one live test ignored by
default, clippy and fmt clean. Independent sweep 20 Aug: 178 registered, 111 servers modelled,
69 recorded non-results (59 GameSpy timeouts). Reachability is ~15% below the 19 Aug snapshot;
treat that as drift until a second sweep says otherwise.

**Status: complete** (commits `34f41d7`, `1660f1c`, `f938ae3`). 29 tests, one live test
ignored by default, clippy and fmt clean.

**Resolved defect — client-count rendering.** `numplayers` (`sv_gamespy.c:164`) is `SV_NumClients()`
and bots are *not* in `svs.clients`, so `clients_reported` and the `minplayers`-derived
simulated count are **disjoint**, not part-and-whole. The CLI renders them as a subset
(`0 (6 simulated)/32 clients`, live on 11 servers) and has a latent `all simulated` branch that
fires when humans happen to equal bots. Fixed in `f938ae3`: `ReportedOccupancy` owns the two
disjoint quantities, `total_occupancy()` returns `None` unless both are known, and the CLI
renders `0 clients (+6 bots) · cap 32`. Verified live on all 11 bot servers. Only OpenMoHAA servers (11/111)
publish `minplayers`; no retail server produced a false simulated count, but the derivation
should be gated on the OpenMoHAA engine string rather than on the field merely being present —
`minplayers` is a standard GameSpy key that means something else elsewhere.

**Known unsolvable, new.** Whether bots consume human slots is **not observable**.
`G_FindFreeEntityForBot` (`g_bot.cpp:215-235`) starts at slot 0 when `sv_sharedbots` is set and
at `maxclients` when it is not, so bots either share the advertised capacity or sit above it.
`sv_sharedbots` (default `0`, `CVAR_LATCH`) is published in **no** reply — 0 occurrences across
111 live servers. So `maxplayers` minus clients is not a reliable count of free slots on a bot
server, and the UI must not imply it is.

## Milestone 3 — content resolution *(here)*

- moh-db client over `/maps`. Case-folded matching; rank one-to-many candidates by
  `mapFileTested`, then `downloads`. `gameType` is accepted and ignored upstream — filter
  locally, do not trust it.
- **Disambiguate on BSP checksum** where the server publishes one (30/127). Where it does not,
  match by name, say so, and verify the `.bsp` paths inside the archive after download rather
  than claiming a match that was never checked.
- PakRadar `filelist.txt` parser — the only source carrying md5, so it is the verified path
  where present (1/127).
- Download to staging, **record** the digest (moh-db publishes none: trust-on-first-use, not
  verification), install with filename preserved. Reject archives containing `.exe`/`.dll`.

**Status: complete** (commit `3dbebff`). 40 tests, two live tests ignored by default, clippy
and fmt clean. Live `resolve 173.249.214.104:12203` reproduces 4 exact / 2 choice-required /
1 no-source, 9.0 MB exact and 23.7 MB with choices. Fuzzy matching is bounded rather than
general: strip an `obj_` prefix or `_obj` suffix from the basename, then require exact basename
equality — no edit distance anywhere. Integrity is separated at the type level
(`MohDbIntegrity::RecordedSha256` vs `PakRadarIntegrity::VerifiedMd5`), so no moh-db path can
say "verified".

**Confirmed upstream limitation.** `gameType` is not merely ignored by the endpoint — it is
absent from the public `MapDto` entirely, so local pre-download game-family filtering is
impossible. Verified directly against the API. Post-download BSP inspection is the substitute.
Ranking inputs are sound: `downloads` is populated on 2000/2000 sampled records and
`mapFileTested` on 1465.

**Deliberate asymmetry, worth a comment in the code.** `inspect_archive` hard-rejects an entire
archive when any BSP entry fails to parse, the opposite of M1's per-entry skip. This is correct,
not inconsistent: M1 scans the user's own install, where one junk file must not destroy the
index; M3 decides whether to write a stranger's archive into the game directory, where "some
entries did not parse" is exactly when to stop.

**Open — Windows filename hardening.** `validate_package_filename` rejects separators, `:`, `.`,
`..` and non-`.pk3` names, but not Windows reserved device basenames (`CON`, `PRN`, `AUX`, `NUL`,
`COM1`-`COM9`, `LPT1`-`LPT9`) or trailing dots and spaces, which Windows silently strips — so
`evil.pk3.` and `evil.pk3` collide and defeat `persist_noclobber`. The filename is third-party
data from moh-db and is written to disk on Windows in M5/M6. Testable now as a pure string check.

**Verify:** the real `<[TFC]> Sniper Only OBJ` rotation — 14 maps, 7 present, 7 absent, and the
absent set resolving to 4 exact catalogue matches, 2 near-matches needing a choice
(`obj/questufou_s_yvette_obj` → catalogued as `dm/Questufou_s_Yvette`), and 1 with no source at
all (`obj/obj_morning2`). One live server exercises all three outcomes.

## Milestone 4 — the join, headless *(here)*

- Compatibility gate emitting the four reviewed states: **Compatible / Needs N maps /
  No source / Can't tell**. "Compatible" means nothing checkable is wrong — bans, capacity and
  ping gates are decided in `SV_DirectConnect` and are **not** predictable, so they are never
  folded into a preflight verdict.
- `+connect host:port` command construction with the right profile and `fs_game`.
- `droperror` string table → player-facing copy, from `code/server/sv_client.c`.
- `reveille-cli` runs the whole pipeline end to end. This is the real proof the architecture
  works, and it lands before any GUI exists.

**Status: complete** (commit `a97d8f7`). 48 tests, two live tests ignored by default, clippy and
fmt clean. `classify` takes preflight and resolution as separate inputs, so `Can't tell` needs no
catalogue call; `NoSource` requires every needed map to resolve conclusively to no source.
Live sweep against the full local install: 113 classified = 83 Compatible + 15 Needs maps +
0 No source + 15 Can't tell. Cross-checked — the 15 `Can't tell` are exactly the 15 servers
publishing no `sv_maplist` (98 + 15 = 113).

**Open, CLI polish.** `browse` hides the state it computes: per-server rows show no compatibility
state even when `--path` is given, and `--format json` omits the assessment entirely — only the
aggregate counts appear, in text. This is not an M6 blocker (the Tauri shell links
`reveille-core` and calls `classify_server` directly; `CompatibilityAssessment` already derives
`Serialize`), but `reveille-cli` is the artefact that demonstrates the pipeline, so it should
show it. `join` surfaces the state correctly, including
`Can't tell — server did not publish a rotation`.

**Open, test gap.** `NoSource` is 0/113 on live data, so `compatibility_states.json` is its only
coverage. The fixture covers all four states and the partial case (TFC: 6 resolvable + 1
no-source → `needs_maps`), but not the `non_results.is_empty()` guard. Low risk — the
`resolutions.len() == count` check catches the same case — but it is the one path where the badge
could lie.

**Finding for the PRD — measured under control.** Classifying **one frozen 114-server set**
against both indexes, so only the index varies: the 88-map install yields 84 Compatible / 15
Needs maps / 15 Can't tell; a **stock Pak0–Pak5 retail install** (54 maps) yields 83 / 16 / 15.
Exactly **one** server flips (`-<MisFits>- Rifle/sniper`). An independent Python reimplementation
of the gate produced these numbers, agreeing with the Rust classifier. So a newcomer who has
installed nothing beyond retail already reaches ~73% of the live population, and content
resolution is the last mile for ~14% of servers rather than the primary blocker on the ten-minute
criterion. Do not re-derive this from two separate live sweeps — the master churns by more than
the effect size (`registered` moved 169 → 210 within this session).

## Cross-platform posture

The PRD's deferred list says *"macOS and Linux builds — Windows first; the core crate stays
portable so this is deferred, not precluded."* That remains the decision, and the code has kept
faith with it: M1–M4 carry **no** Windows dependency, no `cfg(target_os)` anywhere, and
`LaunchCommand` takes `program` from the caller rather than baking in an `.exe` name. All of it
runs on aarch64 Linux today.

What is actually Windows-specific is narrow: M5 install *discovery* (registry, GOG Galaxy, EA App
roots) and M6 shipping (SmartScreen, signing). Tauri itself is cross-platform.

**Decision, 19 Aug 2026: v1 is Windows only.** Considered shipping Linux alongside it — it is
much the cheaper of the two deferrals — and decided against widening v1's surface while the
Windows signing question is still open. Recorded below so the reasoning survives, because the
question will come back.

**Linux and macOS are not the same cost, even though both are deferred.**

- **Linux would be nearly free** whenever it is picked up. There is no native retail MOHAA for
  Linux, so discovery is a Wine/Proton prefix scan or — realistically — the user-picked folder
  fallback that has to exist anyway. Packaging is a Tauri `.deb`/AppImage. No notarisation.
- **macOS is the expensive one.** Apple notarisation and a paid developer account, on top of the
  Windows signing decision that is already open.

**OpenMoHAA ships for every target that matters** (checked 19 Aug 2026, `v0.82.1`): linux
amd64/arm64/armhf/i686/ppc variants, `macos-multiarch-arm64-x86_64`, and windows x64/x86/arm64 as
both `.zip` and `.msi`. Note `linux-arm64` — the engine runs on this very machine, so if Linux is ever
promoted into scope it could be dogfooded here rather than only on the Windows box.

**Release integrity — correction to the M5 wording.** There is **no** `SHA256SUMS` asset and no
digest in the release body. GitHub's API instead returns a per-asset `digest` field
(`sha256:57692d05…`). That is the thing to verify against. Be honest about what it buys: the
digest arrives from the same origin as the download, so it defends against corruption and a bad
CDN edge, not against a compromised GitHub account. Asset selection is an `(os, arch)` → asset
name mapping, which is portable logic and belongs in the core crate, not the platform layer.

**macOS OpenMoHAA overlay, 1 Sep 2026.** The Windows-only v1 *shipping* decision stands: GitHub
Releases still attach NSIS, and there is no Apple notarisation. What is no longer deferred is
installing the official `macos-multiarch-arm64-x86_64` zip into a user-picked folder that already
has `main/` (the original pk3s stay), launching `launch_openmohaa_*` rather than
`openmohaa.exe`, and treating `~/Library/Application Support/openmohaa` as the home path. The
Tauri bundle list includes `app` and `dmg`; those are built on a Mac with `just bundle-macos`,
not by the Windows release workflow. NSIS is not a Mac installer.

## Milestone 5 — Windows platform layer *(split)*

- Install discovery: registry `Uninstall` keys, GOG Galaxy manifests, **EA App / Origin**
  install roots, common literal paths, user-picked folder fallback.
- **Steam is not a distribution path and must not be built for.** MOHAA is not sold on Steam;
  EA moved its catalogue to Origin around 2011 and War Chest is GOG-only among the big stores
  today (plus EA's own app). A `libraryfolders.vdf` walk would serve nobody. Checked 19 Aug 2026.
- **EA App / Origin is a real path the plan previously omitted.** EA sells War Chest on its own
  store and lists it under EA Play. Caveat reported by users: the Spearhead and Breakthrough
  expansions do not launch through the EA app. It means an EA App install may be AA-only, and
  `install::identify` must report that from the data directories rather than assume War Chest
  implies all three. **Status, 26 Aug 2026: Spearhead and Breakthrough are in v1** — see
  "Spearhead and Breakthrough, promoted into v1" below — so this caveat now decides which of the
  three games such an install can offer, rather than being a note about a deferred feature.
- Seed the hash→version corpus from real GOG, EA App and retail-disc installs.
- OpenMoHAA install and update from GitHub Releases. Never silently overwrite a running binary.
  Integrity comes from the API's per-asset `digest` field — see the correction above; there is no
  `SHA256SUMS` asset to read.
- Launch the client. Retail 1.11/1.12 launch behaviour on Windows is an **assumption** — it has
  only ever been exercised against OpenMoHAA on Linux — and is the first thing to test. Log
  tailing is cut from v1; see above.

**Part A — here, on this machine.** Portable logic with synthetic fixtures, provable by
`windows-latest` CI: GOG Galaxy manifest and registry-`Uninstall` *layout* parsing, EA App /
Origin root parsing, the release-asset selector, and the GitHub Releases client with its digest
check. **Status: complete** (commit `438550f`). 57 tests, three network tests ignored by default,
clippy and fmt clean.

### Engine facts the Windows work needs (verified in openmohaa source, 19 Aug 2026)

- **GOG's registry key is in the engine already.** `Sys_GogPath` (`code/sys/sys_win32.c:191-219`)
  reads `HKEY_LOCAL_MACHINE\SOFTWARE\GOG.com\Games\1441704920`, value `PATH`, opened with
  `KEY_WOW64_32KEY`. `1441704920` is the War Chest product id.
- **The engine's Steam support is vestigial ioquake3.** `Sys_SteamPath` (`sys_win32.c:136-137`)
  carries `STEAMPATH_APPID "2200"` — Quake 3 Arena. Independent corroboration that Steam is not a
  MOHAA path.
- **Client log tailing is cut from v1** (decision by the project owner, 19 Aug 2026). The client
  already shows the player whatever the server said, so tailing a log to repeat it buys little
  and costs a platform-specific log-path hunt plus an unknown retail-versus-OpenMoHAA divergence.
  `explain_rejection` in `join.rs` stays — it is correct, tested against all nine `sv_client.c`
  sites, and is the v2 starting point — but it has no input in v1 and must be labelled as not
  wired up. Consequences: drop `+set logfile 2` from the launch arguments, and do not implement
  log-path resolution or tailing.

  <details><summary>Log facts, retained for v2</summary>

  `com_logfile` is `Cvar_Get("logfile", "0", CVAR_TEMP)` (`common.c:1909`), so the log is off
  unless asked for; values above 1 force an unbuffered flush (`common.c:288-293`), and plain
  `logfile 1` buffers so a tail sees nothing until exit. `FS_FOpenFileWrite_HomeData` builds
  `<fs_homedatapath>/<fs_gamedir>/<name>` (`files.cpp:1027-1030`); `Sys_DefaultHomePath`
  (`sys_win32.c:97-120`) gives `%APPDATA%\openmohaa` — **corrected 26 Aug 2026**, see
  `engine-facts.md` §3b. Three header lines come from
  shared qcommon code (`common.c:285-288`) and would appear in a client log too:
  `logfile opened on`, `=> game is version`, `=> targeting game ID`. **No client log has ever
  been observed in this project** — the one inspected was the dedicated server's, under a custom
  `fs_homepath`. Client-side body content remains entirely unverified.

  </details>

- **Home path outranks the install directory in the search path.** `FS_InitPathVars` registers
  homeconfig, homedata, homestate, then basepath, apppath, steampath, gogpath,
  microsoftstorepath; `FS_AddGameDirectories` walks that array **in reverse** and
  `FS_AddGameDirectory` prepends, so the first registered wins (`files.cpp:3245-3257`,
  `3534-3572`).

  **Install target: the game directory first, the home path only as a fallback.** An earlier
  draft of this plan said always write to the home path. That was over-corrected. Dropping
  pk3s into `<install>\main\` is what the community does, what every guide says, and where a
  user expects to find and delete them later — confirmed by the project owner from their own
  Windows machine. It also works on retail 1.11/1.12, which predates the home-path split and has
  no home path at all. So:

  1. Probe whether `<install>\main` is writable — do not infer it from the path string.
  2. If it is, install there. This is the normal case; standalone GOG defaults to
     `C:\GOG Games\...`, which needs no elevation.
  3. If it is not — the `C:\Program Files (x86)` case — fall back to
     `%APPDATA%\openmohaa\<game directory>` on OpenMoHAA, and say plainly in the UI where the
     files went. Never raise a UAC prompt mid-journey; the ten-minute criterion cannot absorb
     one.
  4. On retail there is no fallback. If the directory is unwritable, that is a real blocker and
     must be reported as one, not worked around silently.

  This keeps `install_archive`'s existing `game_directory` parameter meaningful — Part B supplies
  the resolved target, and the choice of target is the caller's.

- **Caveat, and it is the untested one:** the search-path facts above are OpenMoHAA. Retail
  1.11/1.12 predates the home-path split and has no home path at all, so the game directory is
  its only install target. Test retail launch first on the Windows machine.

### Retail launch measurement (Windows, 19 Aug 2026)

Tested the real retail-disc AA client at `D:\Jeux\EA GAMES\MOHDA\MOHAA.exe` (file version
`1.2.4.190`, SHA-256 `ed028e97cb56ea3a89a821635b07e0ed87bcbab751b6e13e88edc9c02dfc88cc`)
against a local dedicated server. The existing AA `LaunchCommand` argument vector
(`+set com_target_game 0 +set fs_game "" +connect 127.0.0.1:12233`) launched the client and the
client visibly attempted the connection. Thus retail accepts `+connect host:port`, including
when it is passed with the command shape constructed in milestone 4.

The profile-selection assumption was only half right. Binary inspection finds `fs_game` in the
retail AA and Spearhead executables, and the empty AA value did not prevent the connection
attempt: it is the retail mod-directory selector as expected. `com_target_game` is absent from
both retail binaries, however; it is an OpenMoHAA cvar (`common.c:3229-3235`) and cannot select a
retail product. Retail uses separate executables (`MOHAA.exe`, `moh_spearhead.exe`, and
`moh_breakthrough.exe`). The Windows launch layer must therefore select the retail executable
from the requested profile and must not rely on `com_target_game`; OpenMoHAA continues to use
one executable plus `com_target_game`. This is a launch-dialect difference, not an install-target
change: retail still has no home path, so `<install>\main` remains its only AA content target.

**Part B — Windows only.** Registry *enumeration*, seeding the hash→version corpus from real GOG,
EA App and retail-disc installs, launching the Windows client, and the untested retail 1.11/1.12 launch
assumption.

### Review of Part B (from the Linux machine, 19 Aug 2026)

- **`com_target_game` finding confirmed against the engine.** `Com_InitTargetGame`
  (`common.c:3228-3235`) creates it alongside `com_target_demo`/`com_target_version` as part of
  OpenMoHAA's `target_game_e` multi-target abstraction. Retail shipped three separate
  executables and has no such cvar. The retail/OpenMoHAA launch-dialect split is right.
- **Linux build verified here** — 61 tests, `clippy -D warnings`, and `fmt --check` all clean on
  aarch64 Linux, which the Windows machine cannot check. `crates/reveille-cli/src/windows.rs`
  compiles everywhere because it only uses portable `std`; `mod windows;` is not `cfg`-gated, so
  the name is misleading off-Windows, but nothing breaks.
- **Resolved in milestone 6 — launch dialects no longer share positional slicing.**
  `LaunchCommand::arguments_for` now constructs each dialect explicitly. Retail cannot inherit
  or lose an argument merely because the OpenMoHAA vector changes position or length.
- **The corpus gap is not a blocker.** Seeding GOG and EA App hashes is an ongoing activity, not
  a Part B deliverable. `IdentificationMethod::RecognizedBinaryUnknownHashes` exists precisely so
  an unmeasured binary still identifies honestly, and the PRD states the corpus starts empty and
  falls back to the build string. Refusing to invent entries was right; gating the milestone on
  them was not. Buying the GOG copy would seed that half whenever convenient.

**Part B status (19 Aug 2026): implemented; hash corpus holds retail only.** Live
32- and 64-bit `Uninstall`/Origin enumeration and the engine's 32-bit GOG key are implemented and
exercised against this Windows machine's real hives. The hives contain no EA App or GOG MOHAA
install (both correctly report no result). Retail/OpenMoHAA launch dialects, process launch, and
the probed install-target policy are implemented. The real French retail-disc AA 1.11, Spearhead
2.15, and Breakthrough 2.40 binaries on this machine seed the corpus and identify through
`KnownBinaryHashes`. No GOG or EA App binary was available to measure, so those hashes have not
been invented. That is a corpus gap to fill opportunistically, not a milestone blocker.

Report Part A complete on its own rather than holding the milestone open for the move.

## Milestone 6 — shell and journeys *(Windows)*

- Tauri shell implementing the three designed screens (first run, server browser, join
  preflight), newcomer-first with power features one layer down.
- Journeys A, B and C end to end.
- **Working end to end on the owner's Windows machine.** That is v1's bar (decision, 19 Aug
  2026). Not packaged, not signed, not distributed.

**Status: complete (19 Aug 2026).** `reveille journey` composes detection, identification, a
complete live browse, preflight, catalogue resolution, safe archive installation, a rescan, and
process launch in one command. The first live pass found 114 answering servers and 94 recorded
non-results, classified `216.146.25.240:12203` as Compatible, and launched OpenMoHAA. A second
pass exercised the content branch against `62.194.57.8:12205`: the server repeated
`dm/aftermath` seven times in its rotation, exposing a real seam. Preflight now deduplicates by
the engine-normalized map key, truthfully reports Needs 1 map, installed the exact verified
`aftermath.pk3` into the probed writable `main` directory, rescanned to Compatible, and launched
OpenMoHAA to the server. The shared discovery pass now records later duplicate authoritative
game endpoints as non-results rather than letting any surface double-count them.

The Tauri shell links `reveille-core` directly and implements first-run detection/manual folder
selection, the live server browser, and join preflight. It carries the four state names unchanged,
sorts only by reported human clients, renders bots separately and additively, offers explicit
choices without auto-applying an ambiguous result, records per-map failures, and displays the
actual destination whenever OpenMoHAA falls back to its home path. The development shell
builds and opens on this Windows machine. Default tests remain offline: 66 pass and the three
live network checks remain ignored; workspace clippy and fmt are clean.

**Signing and distribution: deferred to shipping, by decision.** The chosen channels are winget
and possibly the Microsoft Store, which is consistent with what the blueprint already argued —
let the package manager carry the reputation rather than buying a certificate. Store submission
signs the package as part of the process, so that channel resolves signing rather than deferring
it. Nothing here blocks M6.

> **Re-check the install target if the Microsoft Store channel is taken.** Store distribution
> means MSIX, which sandboxes filesystem access; writing pk3s into `C:\GOG Games\...\main\`
> from a packaged app may be virtualised or refused. The probe-then-fall-back policy should
> survive it, but it has not been tested under MSIX and should be before a Store submission.
> winget carries no such constraint — it just fetches a publisher-hosted installer.

**The ten-minute timed test moves from v1 to ship.** *"Minutes from valid game assets on disk to
being in a game with other people, measured on someone who has never played MOHAA"* remains the
product's success criterion, but it cannot be run against something that only works on one
machine. v1 proves the pipeline composes; the timed test gates shipping. Keep building toward it
— the newcomer-first screen design, the four honest states, no jargon — because retrofitting
those after a developer-shaped v1 is how launchers end up developer-shaped.

### Review of Milestone 6 (from the Linux machine, 19 Aug 2026)

**M6 is accepted against its stated bar.** Commit `d4812eb` adds the Tauri shell, the composed
`journey` command, and the two fixes carried over from the Part B review. Both journeys were
demonstrated end to end on the owner's Windows machine, which is what v1 asked for.

**Both previously open items are genuinely closed.** `arguments_for` no longer slices by magic
index — each dialect builds its own vector, so inserting an argument can no longer silently
corrupt the retail form. `browse` now shows the compatibility state it computes: a per-row badge
in text and the full `CompatibilityAssessment` per server in JSON, closing the M4 CLI-polish
note.

**Rotation dedup is correct and does not disturb the frozen measurements.** `preflight::check`
now keys on `MapKey` and keeps the first spelling and position, so a rotation listing the same
map twice needs it once. Entries that fail to normalise are never deduped, which is the safe
direction. Dedup can only shrink a `count`; it can never flip a map between present and absent,
so the 84/15/15 and 83/16/15 index-comparison figures stand unchanged.

**Confirmed defect — ubuntu CI has been red since this commit.** `d4812eb` added
`crates/reveille-app` to the workspace members, and CI ran `cargo test --workspace` on
`ubuntu-latest` with no GTK or WebKit development packages, so `glib-sys`'s build script fails at
`pkg-config --libs --cflags glib-2.0`. Run `32269538771`, the first failing run in the project's
history. Reproduced locally: `cargo check -p reveille-app` on this aarch64 Linux machine fails
the same way at `gdk-3.0`. The `windows-latest` leg of that run was **cancelled by the matrix's
default `fail-fast`**, not failed, so it proved nothing either way about the shell.

Fixed by splitting the matrix into two independent jobs: a `portable` job that tests, lints and
format-checks `reveille-core` and `reveille-cli` on ubuntu — the invariant that actually matters,
since those two crates carry the deferred Linux and macOS builds — and a `windows` job that runs
the whole workspace including the shell. Separate jobs also mean neither leg can cancel the
other. Installing GTK and WebKit on the ubuntu runner was considered and rejected: it buys a
Linux build of a crate that is Windows-only for v1, at the cost of an apt step that will rot.
Both jobs are green on `b99eac8`, so the Tauri shell is now built by CI on Windows and not only
on the owner's machine.

**Resolved after review — duplicate registrations remain evidence without inflating servers.**
`discovery::browse` sorts by master endpoint, retains the first complete result for each
authoritative `(address, game_port)`, and demotes each later one to a recorded
`DuplicateEndpoint { game_port }` non-result. `registered`, `inspected`, and the duplicate's
parseable GameSpy reply remain visible; every figure derived from complete servers now counts the
game endpoint once. Different game ports on one address remain distinct. Both caller-side
filters were deleted, so the CLI browse, journey, aggregate classification, and app all consume
the same invariant.

**Resolved after review — OpenMoHAA arguments have one source of truth.**
`LaunchCommand::new` first constructs the typed command and then derives the serialized/display
`arguments` field through `arguments_for(LaunchDialect::OpenMohaa)`. The invariant has an explicit
test, while the retail dialect remains independently tested.

**Resolved after review — Windows policy is shared without entering `reveille-core`.** The new
`reveille-platform` crate owns client detection, product-specific executable selection, probed
install-target resolution, and process launch. Both executable crates depend on it; their two
partial test sets were merged and extended. Filesystem mutation and process spawning are gated
inside the crate for Windows, while an explicit unsupported result keeps the portable CLI
buildable on other targets. The Windows workspace now has 67 passing default tests with the
three live network checks still ignored; the locked portable package selection runs 64 of those
tests. Workspace and portable clippy, fmt, and the shell's JavaScript syntax check are clean.

**Verified as sound — the indexed install and the launched binary are the same install.** Both
`build_preview` and `install_and_launch` derive from a single `install::identify(path)`, so the
directory that is scanned, the directory maps are written into, the launch dialect and the
executable path all descend from one `Installation`. That is what makes "installed
`aftermath.pk3`, rescanned to Compatible, launched" evidence rather than coincidence. Retail
correctly has no home fallback; when it falls back on OpenMoHAA the UI reports
`used_home_fallback`, and the engine's search path puts the home path above the install
directory, so a pk3 written to either is found.

**Scope of this machine's verification.** 63 tests pass with 3 network tests ignored — Codex's
66, reproduced — and `clippy -D warnings` and `fmt --check` are clean, on aarch64 Linux, for
`reveille-core` and `reveille-cli` **only**. The `reveille-app` crate cannot be built here at all,
so none of the GUI half of the 5,257-line diff has been checked from this machine. The shell's
behaviour rests on the owner's demonstration.

### Review of the Milestone 6 follow-ups (from the Linux machine, 19 Aug 2026)

All three items landed as briefed (`3d0d877`). Duplicate game endpoints are now **demoted rather
than dropped** — `ProbeStage::EndpointDeduplication` with
`NonResultReason::DuplicateEndpoint` — so `registered` and `inspected` stay honest while every
`servers`-derived figure, `clients_reported` included, stops double-counting. Outcomes are sorted
before dedup, so which registration is retained is deterministic. `reveille-platform` holds the
shared Windows policy with both formerly divergent test sets merged. `LaunchCommand::new` derives
`arguments` through `arguments_for`, with an invariant test.

**Regression introduced by the extraction, fixed in `98e0317`.** The new crate wrapped
`resolve_install_target`, `probe_writable` and `launch_client` in `#[cfg(windows)]` with
`cfg(not(windows))` stubs returning `UnsupportedOperatingSystem`. Three consequences, all
confirmed here:

1. The stubs carry no `# Errors` section, so `clippy -D warnings` fails on Linux at `lib.rs:115`
   and `:169`. CI run on `3d0d877` is red. Codex's clippy pass was clean because on Windows those
   stubs do not compile — the portable leg added in `b99eac8` is what caught it.
2. `reveille-cli journey` died before touching the map index, so the composed pipeline could no
   longer be exercised on the machine the plan assigns that role to.
3. `writable_game_directory_is_preferred_without_a_fallback` became `#[cfg(windows)]`, silently
   dropping probe-and-fallback coverage from the ubuntu leg — the one piece of policy that has
   already needed correcting twice.

None of it needed the gate: `probe_writable` is `OpenOptions::create_new`,
`resolve_install_target`'s only Windows-specific call is `env::var_os("APPDATA")` which already
degrades to `MissingAppData`, and a failed spawn reports a specific `io::Error`, which beats a
blanket refusal. **Keep this crate portable.** Windows is the only *supported* platform in v1;
that is a policy statement, not a reason to make the code unbuildable elsewhere.

**Journey verified end to end on Linux against the real corpus.** `journey --path /home/feho/MOHAA
--client-kind retail 173.249.214.104:12203` browsed, deduped, preflighted at *needs 7 maps*,
installed 4 archives, re-scanned to *needs 3*, and stopped at `Launch ready:` because `--execute`
was absent. The 4-of-7 split reproduces the frozen TFC finding exactly: 4 exact matches, 2
requiring a choice, 1 with no source.

> **Caution for anyone repeating that command.** `/home/feho/MOHAA/main` is the live dedicated
> server's game directory and the frozen fixture behind `real_corpus.rs`. Installing into it makes
> the 88-map and 7-of-14 assertions fail. Point `--path` at a copy.

### Interface redesign (19 Aug 2026)

**The shell's interface was rebuilt from scratch, and the design now lives in the repository.**
The previous interface was written by an agent that could not fetch the UI mockups linked from
`AGENTS.md`, and without them it produced a marketing page in a launcher's clothes: a welcome hero,
a three-step promise grid, 75px serif headlines, a film-grain overlay, and a four-column table that
discarded almost everything the pipeline knows. The root cause was the design living only at a URL,
so the first fix is [`docs/ui.md`](ui.md) — authoritative, offline, and sufficient to rebuild the
interface with no network access.

**Decision — the server list shows cost, not a verdict.** The obvious design is a four-state status
column: green Compatible, amber Needs N maps, red No source, grey Can't tell. It was rejected. A
traffic light teaches one behaviour, click only green, and that is the wrong behaviour here: on the
19 Aug corpus 15 of 113 classified servers need maps and 15 publish no rotation, so a green/amber
column pushes a player away from about a quarter of the live population — and specifically away
from the servers with the richest custom rotations. That reproduces, inside Reveille, the "the game
is dead" impression Reveille exists to correct. The Needs column therefore prices the work
(`+ 7 maps`) in default ink, leaves a ready server's cell **empty**, and colours only `No source`,
the one state a download cannot fix. The four canonical state names are unchanged and moved to the
detail pane, where the decision is actually made and there is room to explain them. Verified live:
`<[TFC]> Sniper Only OBJ` reads `+ 7 maps` with no colour anywhere in the list.

**Decision — the join gate is the map running now, not the whole rotation.** `No source` previously
refused the launch outright. That is wrong: a server with one unobtainable map later in its rotation
is playable until the rotation reaches it, and refusing invents a problem the engine does not have.
`join::current_map_readiness` now classifies the server's `mapname` against local content as
`Playable`/`Missing`/`Unknown`, independently of the rotation verdict, and `launch_refusal` refuses
only when the map running *now* is absent — the one case consent cannot buy, because that connection
is dropped on arrival. Everything else launches on explicit consent, with the consequence stated:
*"you will be dropped when the rotation reaches this map."* Four unit tests pin the gate.

**Consent is now explicit.** The old shell derived `allow_unchecked` silently from the state, so a
player could launch an unchecked `Can't tell` join without ever being told. It is a visible toggle,
and the command parameter was renamed `accept_incomplete` to match what it means.

**Progress, cancellation and structure were added to the pipeline to support it.**
`discovery::browse_streaming` reports each probe as it lands and stops when the receiver is dropped
— cancellation with no token type and no new dependency. Duplicate-endpoint demotion still runs at
the end, so streamed rows are explicitly pre-deduplication and the returned report stays
authoritative. `MohDbClient::resolve_all_reporting` reports each catalogue lookup, and
`download_mohdb_archive_reporting` streams the body to staging with byte progress instead of
buffering whole archives in memory. The app emits `reveille://browse`, `reveille://preview` and
`reveille://install`, caches the preview so a launch no longer repeats the moh-db pass it just ran,
and returns `BrowseSummary` plus a per-reason non-result breakdown rather than a bare count.

**Two defects were found by running it, not by reading it.** The Runs column rendered
`server.version` — "Medal of Honor Allied Assault 1.11 win-x86 Mar 5 2002" — which truncates to
"Medal of Honor Allied" in every row and distinguishes nothing; it now uses `game_version`
(`1.11`, `1.12+0.83.0`). And rebuilding the toolbar on every state change detached the search
input mid-keystroke, so the field accepted exactly one character; the toolbar is now built once and
updated in place, with `preserveFocus` guarding the panes that do repaint.

**Defect found by the owner after the redesign landed, and fixed.** The action bar hard-blocked the
join whenever the map running now was absent, which meant a server running a map the player did not
have offered no way to *get* that map — the one screen that could have solved it disabled its own
button. The backend was never wrong: `install_and_launch` downloads, rescans and only then calls
`launch_refusal`, so a fetchable current map is present by the time the gate runs. The interface was
pre-empting it. `currentMapFetchable` now blocks only when the catalogue has no source for the
running map, and otherwise says plainly that fetching the files is what makes the join work.
Reproduced and fixed against `[FR]Les Vieux Raleurs` running `obj/obj_frag-n-rock`.

**Verified live on the owner's Windows machine** against `D:\jeux\EA GAMES\MOHDA` (main, mainta,
maintt; retail 1.11 and OpenMoHAA present). Detection reported the install as verified against a
known binary hash with all three products listed. A full sweep answered 106 of 190 with 108 bots
counted separately and 84 recorded non-results broken down by reason. `<[TFC]> Sniper Only OBJ`
reproduced the frozen fixture exactly: 7 of 14 maps present, 4 exact matches at 9.1 MB, 2 requiring
a choice, 1 with no source; selecting a candidate moved the total to 13.9 MB and the copy to "1 map
still needs a choice". The install-and-launch step was left to the owner rather than writing into a
live game folder unasked.

**Second review by the owner, five issues, all fixed.**

*The Stop button did nothing.* It was rebuilt on every render, and a sweep notifies several times a
second — so the element was replaced between the player's mousedown and mouseup and no `click` event
was ever dispatched. This is the same defect class as the one-character search field and was missed
because the fix at the time was applied to the toolbar's inputs, not to its buttons. Both browse
controls are now built once and shown or hidden; the toolbar creates no element after boot. The
cancellation path itself was correct, but two gaps were closed while there: a stop pressed just as a
sweep ended left a `Notify` permit behind that would cancel the *next* sweep before it probed
anything, so the permit is drained at the start of each browse; and the ~2.5 s during which probes
already in flight drain now reads "Stopping…" rather than leaving Stop looking inert.

*Selecting a server while the list was still loading answered "This server is no longer in the
current list."* Streamed rows were offered to the player immediately but `AppState.servers` — what
`preview_join` and `install_and_launch` look up — was only populated when the sweep finished. Every
answered server is now pushed into the shared list as it arrives, and the authoritative
post-deduplication list still replaces it at the end.

*The folder shown in the setup field was `\\?\D:\Jeux\EA GAMES\MOHDA`.* `displayPath` strips the
Windows extended-length prefix and was applied everywhere the path is *displayed*, but the editable
input was seeded from the raw canonicalised `install.root`.

*Too much explanatory text.* The interface argued its reasoning at the player: a paragraph justifying
each rotation group, a standing note about bans and capacity on every server, the no-auto-apply
policy restated at every ambiguous match. Every one of those sentences was true and none of them
changed what the player does next. The explanations that survive are the ones that do; the rest
moved to `title` attributes on the thing they explain, or out of the interface entirely. The rule is
now written down in `docs/ui.md` §9, because "be honest" had been read as "explain everything" and
the two are not the same.

*The consent checkbox was one click too many.* Requiring "Join after fetching what is missing" to be
ticked before the join button enabled asked the player to agree to something stated directly above
it and then act on it. Consent is now the click itself, and the button's label is what makes it
informed: `Join`, `Get 9.1 MB & join`, `Join without a rotation check`, `Join anyway`. The thing
that must not return is the *silent* inference the first shell did — deriving `accept_incomplete`
from the state without changing anything the player could see.

Re-verified live against the same install: the sweep answered 106 of 195, Stop reported "stopped
early", a needs-maps server previewed mid-sweep without error, and `<[TFC]> Sniper Only OBJ` again
reproduced 7 of 14 present with 9.1 MB across 4 files and 2 awaiting a choice.

**Third review, two issues, both fixed.**

*Only the rotation was checked, not the map running now.* `classify_server` preflighted
`server.rotation` and nothing else, so a map the server was running but had not listed in
`sv_maplist` was invisible: it did not count towards `Needs N maps`, it never entered the shopping
list, and `current_map_readiness` returned `Unknown` because the preflight held no entry for it.
Worst case was a server publishing no rotation at all — the shell reported `Can't tell`, showed
nothing to do, and offered a join that the engine would have dropped on arrival. The preflight now
covers `sv_maplist` ∪ `mapname`, deduplicated by `MapKey`, so the running map counts, resolves and
downloads like any other. The rotation verdict is unchanged in kind: a server that published no
rotation stays `Can't tell`, because one checked map is not a rotation check — but its readiness is
now known and its map is fetchable, and the detail pane heads the section **Maps** with "No rotation
published. Only the map running now was checked."

Live: `=MB= Revival Mie` publishes no rotation and was running `dm/dm_stanalie`, absent locally.
Before, it read "not published" with nothing to do. It now prices `+ 1 map` in the list and offers
"Get 5.0 MB & join".

*Clicking a column sorted the rows but the arrow and highlight stayed on Map now.* The header cells
were built once and read `state.sort` at construction, so both froze on whatever the saved sort was
at boot. They are now written in place on every render, the same rule the toolbar already follows
and for the same reason.

**Fourth review, one request: the launch line.** The owner asked that the game start with
`+set ui_console 1 +set cl_playintro 0`. Both cvars now sit in `arguments_for` for both dialects,
between `fs_game` and `+connect`. `ui_console 1` keeps the console reachable, which matters here
because the engine's own message is the only account of a connection Reveille cannot see fail —
everything the launcher checks happens before the process starts. `cl_playintro 0` removes the
intro movies from between the click and the server. `+connect` stays last: it initiates the
connection, so nothing may follow it. The CLI's `render_launch_command` fixtures moved with it.

---

## Add Reborn as an engine choice *(Windows)*

**Status: implemented (23 Aug 2026).** Setup now asks how the player wants to run the game and
passes a typed `original` / `openmohaa` / `reborn` choice through browse, preview, cached-preview
identity, content installation, and launch. OpenMoHAA keeps its argument dialect and writable home
fallback; Original and Reborn use the retail dialect and game directory. The CLI's existing
automatic fallback remains compatible and accepts Reborn as an explicit client kind.

Selection is stored in app preferences keyed by the canonical installation root, so choosing
OpenMoHAA does not require writing into a Program Files installation. A valid saved choice wins;
an unavailable saved choice blocks rather than falling back. With no saved choice, the sole
installed community engine wins, Original wins when neither is installed, and two installed
community engines require an explicit choice. `.reveille-engines/state.json` remains beside the
managed files and records Original/Reborn package identity and hashes.

Legacy Reborn 1.12 player packages are pinned to official documentation commit
`15451e40274e718870dcf8ba295bb8fcde745857`. Frozen size/digest metadata selects `aa`, `aa_sh`,
`aa_bt`, or `aa_sh_bt` from detected data directories. ZIP contents must be exactly the expected
retail-named executables. Reveille preserves first-seen originals under
`.reveille-engines/original/`, stores verified Reborn files under `.reveille-engines/reborn/`, and
transactionally activates either set at the canonical filenames. Process detection covers all
three retail/Reborn executable names and treats an unknown query as blocking. The newer signed
`mohreborn/releases` repository had no published releases when rechecked and remains future work.

**Correction from the real archives.** The proposed `aa_sh_bt` digest contained one wrong nibble:
`425cbe3a4256…`. Downloading the 2,357,315-byte file directly from the immutable commit and hashing
it produced `425cbe3a4253f62b9f088c7715d393b17b929b56631f230ebd99de88d45be457`.
The frozen fixture uses the measured value; retaining the proposed value would reject the official
archive on every install.

Default tests freeze all four mappings and cover metadata, size/digest rejection, unsafe and
unexpected ZIP entries, missing executables, process parsing, choice persistence, first backup,
no-clobber reinstall, switching both directions, and process gates. Live installation is restricted
to a scratch game root; a real player installation is never an acceptance-test target.

**Portable CI correction (23 Aug 2026).** The first Reborn run and the following Ping-column run
both passed the Windows workspace job but failed the ubuntu portable job under `clippy -D warnings`.
Windows-only `tasklist` imports and parser helpers in `reveille-platform` were compiled but unused
in a normal non-Windows library build. Their `cfg` guards now match the existing Windows/test-only
parser boundary; the public conservative non-Windows activity result remains available.

---

## Verification overall

Every milestone is checked against measurements already taken this session rather than against
itself: 88 maps / 0 multi-provider, the two known checksums, the TFC rotation's 7-of-14 split
and its three distinct resolution outcomes, and the 19 Aug population snapshot.
`cargo clippy -- -D warnings` and `cargo fmt --check` clean throughout. CI has run on ubuntu
and `windows-latest` from commit 1; from `d4812eb` the ubuntu leg covers the portable crates and
the Windows leg covers the whole workspace, for the reasons in the M6 review.

## Spearhead and Breakthrough, promoted into v1

**Decision, 26 Aug 2026, by the project owner.** Allied Assault, Spearhead and Breakthrough are
all three v1. Earlier notes tag SH/BT as after-v1; that is superseded, and the EA App bullet in
M5 now reads as "which of the three an install can offer" rather than a note about a deferred
feature.

Most of the pipeline already carried them, and had from M4: `TargetGame` has three variants with
their index-aligned GameSpy keys, `LaunchProfile` maps them to `com_target_game` 0/1/2 and to
`main`/`mainta`/`maintt`, `install::identify` reports products from the data directories, the
Reborn package selector is already four-way on those products, and the CLI has had `--game` since
M4. What was missing was the shell — which hard-coded `TargetGame::AlliedAssault` in four places —
and one engine fact nobody had needed until now.

**The fact that had to be checked: an expansion adds a directory, it does not replace `main`.**
`FS_Startup` adds `com_basegame` and *then* `fs_basegame` (`files.cpp:3640-3645`), and
`Com_InitTargetGameWithType` sets `fs_basegame` to `mainta` or `maintt` (`common.c:3181,3208`).
So Spearhead searches `mainta` over `main`, Breakthrough searches `maintt` over `main`, and
**Breakthrough never reads `mainta`** — `fs_basegame` holds one directory, so the three chains
are not cumulative. Both front ends had been indexing exactly one game directory, which for an
expansion would have reported every base-game map on a Spearhead server as missing. `MapIndex`
grew `scan_chain`, `LaunchProfile` grew `search_directories`, and `platform` grew
`content_search_path` to resolve that chain against the installation and the home path. Recorded
as rule **H13**.

**Correction found while doing it: the OpenMoHAA home path is `%APPDATA%\openmohaa`, not
`%APPDATA%\moh`.** This plan cited `q_shared.h:47` — `HOMEPATH_NAME_WIN_MOH` — which, with the
`mohta` and `mohtt` defines beside it, is referenced nowhere in the engine. `Sys_DefaultHomePath`
appends `com_homepath`, empty for a non-demo build, and otherwise `HOMEPATH_NAME`, which is
`"openmohaa"` (`sys_win32.c:114-117`, `q_shared.h:81`, `common.c:1769-1771`). The fallback
install target was therefore writing to a directory the engine never searches — a pre-existing
bug on the Allied Assault path too, not something the expansions introduced. Fixed in
`reveille-platform` as one named constant with the citation beside it; details in
`engine-facts.md` §3b. It is worth noting *how* it survived: the citation was to a plausible
define with the right name, and nothing checked that the engine actually used it. **Cite the
reference site, not the definition site**, when both exist.

**Reviewed by Codex, 26 Aug 2026 — a code pass and a UI/UX pass.** It re-derived the four
load-bearing engine claims from the source rather than taking them from this file, and confirmed
all four, including the `%APPDATA%\openmohaa` correction. Eight findings were accepted and fixed
in the same change. Three are worth keeping here, because each is a *kind* of mistake rather than
a typo:

- **A pre-spawn `Path::is_file` check is not the same question as "can this be launched".**
  `Command` resolves a bare program name against `PATH`; `Path::is_file` does not, and the CLI's
  default join client is the bare name `openmohaa`. A check added to produce a *better error* had
  turned a working launch into a refusal. The classification now happens after the spawn attempt,
  from `io::ErrorKind::NotFound`. Recorded under **H14**.
- **A probe answered twice can answer differently.** `install_and_launch` downloaded into the
  preview's destination and then resolved the write target a second time to report it — so a
  folder that was locked during the preview and writable afterwards would have been described as
  the destination when the files were in the home path. H8 is about the directory files actually
  went to, and that is knowable only once. Recorded under **H8**.
- **A rule needs its own edge case tested, not its happy path.** `provides` accepted a folder with
  `mainta` and no `main` — the exact state H13 forbids — because it asked only whether the
  expansion's own directory was there. `Installation::playable` now names the runnable subset and
  both front ends and the interface build their choices from it. The parallel UI finding was the
  same shape: the "switch to Spearhead" guidance could be shown to a folder that has no Spearhead,
  because bookmarks are global and outlive the install they were saved under.

The UI pass changed one decision: **the game is chosen in setup when the folder can run more than
one**, not only in the toolbar. The reason is mechanical rather than aesthetic — Continue starts a
sweep immediately, and the toolbar's switch is disabled while a sweep runs, so setup was pointing
the player at a control that was greyed out at exactly the moment they were told to use it.

**What is deliberately not done.** Nothing tries to infer that an install has Spearhead from a
War Chest product name or an executable; `Installation::provides` asks the data directories and
nothing else (rule **H14**, and `registry.rs::war_chest_name_does_not_infer_missing_expansions`
was already guarding the same principle from the discovery side). And the shell offers no
"all games" list: three master registrations mean three sweeps, and merging them would put rows
in one table that no single client can join.

## Scope decided after v1 was drafted

**Browsable mod and map management is v2, decided 22 Aug 2026 by the project owner.** The goal
stated for it: managing maps and mods should feel like installing extensions in VS Code — browse,
install, enable, disable, update, remove.

Recorded here because it is a *new product surface*, not a refinement of what M3 already does.
The v1 pipeline resolves content **reactively**: one server, at join time, for the maps that
server's rotation needs. A catalogue is the opposite shape — the player browses before they have
a server in mind, and the tool must then track what is installed, what it came from, and what
state it is in. Three consequences worth having written down before the work starts:

- It needs **mod-level metadata** — a name, a version, a description, a dependency on a game
  target — and moh-db is a *map* catalogue. Whether that metadata exists anywhere is the first
  thing to check, not an implementation detail to discover later.
- It needs **installed-state tracking**. v1 deliberately has none: `install_archive` preserves the
  filename and `mapindex` re-derives what is present by walking the disk. Enable/disable/update
  cannot be answered by a disk walk alone.
- Enable/disable touches the engine's search path, where the v1 facts were measured for
  *installation*, not for *toggling*. `fs_searchpaths` precedence (`files.cpp:3245-3257`) will
  need re-reading with that question in mind.

None of this blocks v1. It is written down so that v1 does not accidentally foreclose it — in
particular, do not let the reactive path bake in an assumption that content only ever arrives
because a server asked for it.

**Favorites and launch history are not the v2 catalogue, and must not become its foundation.**
Added 24 Aug 2026. What they store is deliberately the smallest thing that works: an address, the
master's query port, and the name the server had when it was saved. No content state, no installed
state, no compatibility verdict — those are facts about a moment, and re-deriving them is exactly
what the sweep and the disk walk already do. The v2 bullet above needs *installed-state tracking*;
this is not the start of it, and extending `bookmarks.js` in that direction would put a store of
remembered facts in the one place the interface is least able to check them (rule **H12**).

Two things did come out of it that v2 can build on. `discovery::inspect_endpoint` probes one
already-known registration without a master query, which makes a server reachable that the master
never listed — the master's list has never been the population, and until now Reveille could not
join anything outside it. And `check_server` merges its result into the same server list the sweep
fills, so nothing downstream of it needed to learn about a second source of servers.

## Windows packaging and signing (27 Aug 2026)

**The installer is Tauri's own NSIS bundler, not Inno Setup.** The question was asked directly, and
Inno loses on the two things that matter here. Tauri's Windows bundler already detects the WebView2
Evergreen runtime and installs it when a machine lacks it; with Inno that is a script to write and
maintain. And `bundle.windows.signCommand` is invoked by the bundler for *every* file it produces,
which is the only way the app executable ends up signed as well as the installer wrapping it — Inno
signs through a `SignTool` directive that wants a synchronous local signtool, and its generated
`unins000.exe` is the piece most likely to ship unsigned by accident. winget is indifferent between
them (`inno` and `nullsoft` are both first-class installer types with silent switches), so the
channel already chosen gains nothing from the swap. What Inno would buy is wizard pages, Pascal
scripting and install-UI control; Reveille installs one executable and a webview.

Two installer decisions worth stating so they are not re-opened by accident:

- **`installMode: currentUser`.** The app does not need Administrator to run, so the install should
  not ask for it. Writing content into a game directory under `Program Files` or `C:\GOG Games` is a
  separate question, and the probe-then-fall-back write-target policy already answers it at runtime
  rather than by elevating the whole application.
- **No licence page in the installer.** GPL-2.0 does not require acceptance in order to *use* the
  program, and a page that must be clicked past is a step charged to every newcomer. The licence
  ships in the repository and in the bundle metadata.
- **`mainBinaryName: "Reveille"`.** Without it Tauri bundles the cargo artifact under its crate
  name, so the installed program, its Start Menu target and its Task Manager row would all read
  `reveille-app.exe`. Verified by building the bundle both ways.

**`.github/workflows/release.yml` builds it.** A `v*` tag builds the installer and attaches it to a
**draft** release, so publishing stays a deliberate act; a manual run builds the same installer and
leaves it as a workflow artifact, so packaging can be exercised without minting a release. The
installer's SHA-256 goes into the job summary, with the same honesty as the M5 note above: it is
printed by the machine that built the file into a public log, which defends against a corrupted
download or a bad CDN edge, not against a compromised account.

**Self-update, added 28 Aug 2026.** Release builds also produce Tauri's signed updater artifact and
a static `latest.json` beside the installer. The app checks GitHub's
`releases/latest/download/latest.json` endpoint in the background, so a draft is invisible and the
offer begins only when the owner publishes it. Tauri compares semantic versions and requires a
minisign signature; this is stronger evidence than the same-origin GitHub asset digest used for
OpenMoHAA. The exact checked offer stays in Rust until the player chooses **Update and restart**.
The transfer is cancellable; the verified apply is not, and on Windows the updater exits Reveille
before NSIS replaces it. A failed check is silent because it is a recorded non-result unrelated to
the player's current setup or server pass.

The signing key is shipping state, not repository state. Generate it once with Tauri's signer,
store the private key and optional password as GitHub Actions secrets
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and store the shareable public
key as the repository variable `REVEILLE_UPDATER_PUBKEY`. The release workflow refuses to build
without both halves rather than publishing a version existing installations can never verify. It
also requires the `v*` tag, Cargo package, Tauri config and npm package versions to agree; otherwise
a binary could install successfully and immediately offer the same release again.

**Correction to the M6 framing: "let the package manager carry the reputation rather than buying a
certificate" was half right.** The half that holds is *rather than buying* — nothing here costs
money. The half that does not: winget does not remove the unknown-publisher line from the UAC
prompt, and anyone who takes the installer straight from GitHub Releases gets Mark of the Web and
the full SmartScreen path regardless of what winget would have done. Signing and the package
manager are complementary, not alternatives.

**SignPath Foundation is the route to try** (checked 27 Aug 2026). It signs open-source projects for
free from an HSM. The alternatives were re-checked and fail on availability rather than merit: Azure
Artifact Signing (renamed from Trusted Signing in 2026) is $9.99/month and would carry the
maintainer's own name, but individual sign-up is **US and Canada only** — the EU and UK are listed
for organisations — so it is not open to this project's maintainer without incorporating. A
conventional OV certificate has needed FIPS-grade hardware since the 2023 CA/Browser Forum change,
which puts it above €300/year plus a token.

What the Foundation costs instead of money, recorded because none of it is obvious:

- **The publisher Windows shows is "SignPath Foundation", not Reveille and not the maintainer.** The
  certificate is issued to the Foundation. The download page has to say so, in the same voice the
  rest of the interface uses about what it does and does not know.
- **It is not instant SmartScreen clearance.** Reputation accrues; Microsoft's own documentation
  says so for OV signing generally. The genuine advantage is that the certificate is shared, so it
  arrives carrying every other project's download history rather than starting at zero.
- **Conditions that are work, not paperwork.** An OSI licence with no commercial dual-licensing and
  no proprietary component (GPL-2.0-only qualifies); a **Code signing policy** section in the README
  linking SignPath.io and the Foundation; documented Author / Reviewer / Approver roles; a privacy
  statement; MFA on every account with commit access; and the project must **already be released in
  the form to be signed**. That last one sets the order: ship v1 unsigned from the workflow above,
  then apply.
- **A reviewer risk to pre-empt.** Their prohibited list includes "unauthorized system
  modifications", and Reveille writes pk3 files into a game directory outside its own install root.
  It is user-initiated and the app is honest about the destination, but the download page must say
  plainly that installing map content is the point, so a reviewer reads it as the feature.

**Mechanism, so the wiring is not guessed at.** `SignPath/github-action-submit-signing-request`
submits *one* artifact per step, which cannot be what `signCommand` calls — the bundler needs a
command it can run per file. The per-file path is the `SignPath` PowerShell module:
`Submit-SigningRequest -WaitForCompletion -OutputArtifactPath` blocks until the signed file comes
back (600-second default timeout), and `-Origin` carries the repository and build metadata that
GitHub Actions is trusted to assert. Note the interaction with the Foundation's per-release manual
approval: each file is its own signing request, so a release is approved once per signed file — two,
for the app executable and the installer — not once overall.

**If signing lands, the Microsoft Store channel is no longer needed.** M6 kept the Store partly
because submission signs the package for you. With SignPath doing that, the MSIX filesystem risk
recorded above — pk3 writes into `C:\GOG Games\...\main\` being virtualised or refused inside a
packaged app — need not be taken at all. winget plus a signed publisher-hosted installer carries no
such constraint.

## Decisions still open

Maintainer model. The moh-db relationship: worth telling them, and worth asking for published
digests and a `gameType` filter that filters. Distribution, packaging and the signing route are
decided above; the SignPath application itself, the winget manifest, and any Microsoft Store
submission remain owner-run shipping work, outside v1.

## Follow-up, not blocking

The PRD's BSP-checksum bullet describes that work as heavier than it proved to be. Worth
softening once milestone 1 lands — a wording fix.
