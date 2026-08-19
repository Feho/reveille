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
log tailing, SmartScreen and signing, and Journey A end to end — which *is* the success
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

## Milestone 5 — Windows platform layer *(split)*

- Install discovery: registry `Uninstall` keys, GOG Galaxy manifests, **EA App / Origin**
  install roots, common literal paths, user-picked folder fallback.
- **Steam is not a distribution path and must not be built for.** MOHAA is not sold on Steam;
  EA moved its catalogue to Origin around 2011 and War Chest is GOG-only among the big stores
  today (plus EA's own app). A `libraryfolders.vdf` walk would serve nobody. Checked 19 Aug 2026.
- **EA App / Origin is a real path the plan previously omitted.** EA sells War Chest on its own
  store and lists it under EA Play. Caveat reported by users: the Spearhead and Breakthrough
  expansions do not launch through the EA app. That does not block v1, which targets Allied
  Assault multiplayer and already tags SH/BT as after-v1 — but it means an EA App install may be
  AA-only, and `install::identify` must report that from the data directories rather than assume
  War Chest implies all three.
- Seed the hash→version corpus from real GOG, EA App and retail-disc installs.
- OpenMoHAA install and update from GitHub Releases. Never silently overwrite a running binary.
  Integrity comes from the API's per-asset `digest` field — see the correction above; there is no
  `SHA256SUMS` asset to read.
- Launch the client and tail `qconsole.log`. Currently exercised only against OpenMoHAA on
  Linux; retail 1.11/1.12 on Windows is an **assumption** and an early thing to test.

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
- **`qconsole.log` is off by default.** `com_logfile` is `Cvar_Get("logfile", "0", CVAR_TEMP)`
  (`code/qcommon/common.c:1909`), so the launcher must pass `+set logfile 2`. Values above 1 force
  an unbuffered flush (`common.c:288-293`) — with plain `logfile 1` a tailing launcher sees
  nothing until the buffer flushes or the game exits.
- **The log lands in the home path under the game directory, not the install directory.**
  `FS_FOpenFileWrite_HomeData` builds `<fs_homedatapath>/<fs_gamedir>/<name>`
  (`files.cpp:1027-1030`), and `Sys_DefaultHomePath` (`sys_win32.c:97-120`) resolves to
  `%APPDATA%\` + `HOMEPATH_NAME_WIN_MOH` = `%APPDATA%\moh` (`q_shared.h:47`; `mohta`/`mohtt` for
  the expansions). So the Windows default should be **`%APPDATA%\moh\main\qconsole.log`** —
  source-derived, not observed.
- **The log is truncated per run and opens with three anchor lines.** These come from `Com_Printf`
  in `common.c:285-288`, which is shared qcommon code, so a client emits them too:
  `logfile opened on <date>`, `=> game is version <product> <version> <platform> <date>`, and
  `=> targeting game ID <n>`. The first is a reliable "the client I just launched has started"
  marker; the second is a better engine fingerprint than a binary hash for OpenMoHAA and confirms
  which `com_target_game` took effect. A tail must handle the file shrinking.

  > **Correction, and a limit on the above.** The `qconsole.log` inspected on this machine
  > (`/home/feho/.openmohaa/main/qconsole.log`) belongs to the **dedicated server**, not a client.
  > Two consequences. First, it used a custom `fs_homepath` (`~/.openmohaa`), so it confirms the
  > *path composition* `<homepath>/<gamedir>/qconsole.log` but says nothing about the default
  > home-path value. Second, its body — `Client 0 connecting with 5000 challenge ping` and the
  > like — is server-side output. **No client log has been observed anywhere in this project.**
  > The cvar behaviour, the path composition and the three header lines are all shared qcommon or
  > sys code and hold for a client; everything about client-side *body* content is unverified and
  > must be discovered on the Windows machine before anything parses it.

- **Home path outranks the install directory in the search path.** `FS_InitPathVars` registers
  homeconfig, homedata, homestate, then basepath, apppath, steampath, gogpath,
  microsoftstorepath; `FS_AddGameDirectories` walks that array **in reverse** and
  `FS_AddGameDirectory` prepends, so the first registered wins (`files.cpp:3245-3257`,
  `3534-3572`).

  **Install target: the game directory first, the home path only as a fallback.** An earlier
  draft of this plan said always write to `%APPDATA%\moh\main`. That was over-corrected. Dropping
  pk3s into `<install>\main\` is what the community does, what every guide says, and where a
  user expects to find and delete them later — confirmed by the project owner from their own
  Windows machine. It also works on retail 1.11/1.12, which predates the home-path split and has
  no home path at all. So:

  1. Probe whether `<install>\main` is writable — do not infer it from the path string.
  2. If it is, install there. This is the normal case; standalone GOG defaults to
     `C:\GOG Games\...`, which needs no elevation.
  3. If it is not — the `C:\Program Files (x86)` case — fall back to `%APPDATA%\moh\main` on
     OpenMoHAA, and say plainly in the UI where the files went. Never raise a UAC prompt
     mid-journey; the ten-minute criterion cannot absorb one.
  4. On retail there is no fallback. If the directory is unwritable, that is a real blocker and
     must be reported as one, not worked around silently.

  This keeps `install_archive`'s existing `game_directory` parameter meaningful — Part B supplies
  the resolved target, and the choice of target is the caller's.

- **Caveat, and it is the untested one:** all of the above is OpenMoHAA. Retail 1.11/1.12 predates
  the home-path split and most likely writes to the install directory, which is where the UAC risk
  actually lives. Test this first on the Windows machine.

**Part B — Windows only.** Registry *enumeration*, seeding the hash→version corpus from real GOG,
EA App and retail-disc installs, launching the Windows client, tailing `qconsole.log`, and the
untested retail 1.11/1.12 launch assumption.

Report Part A complete on its own rather than holding the milestone open for the move.

## Milestone 6 — shell and journeys *(Windows)*

- Tauri shell implementing the three designed screens (first run, server browser, join
  preflight), newcomer-first with power features one layer down.
- Journeys A, B and C end to end.
- **Timed test on a real person** who has never played MOHAA: ten minutes from assets on disk
  to first spawn, no forum, no Discord, no hints.
- Signing and distribution resolved **before** shipping, not after — an unsigned binary that
  scans drives then opens UDP sockets to dozens of hosts will attract SmartScreen, and the
  primary user is precisely the one who will not click through a blue panel.

---

## Verification overall

Every milestone is checked against measurements already taken this session rather than against
itself: 88 maps / 0 multi-provider, the two known checksums, the TFC rotation's 7-of-14 split
and its three distinct resolution outcomes, and the 19 Aug population snapshot.
`cargo clippy -- -D warnings` and `cargo fmt --check` clean throughout; CI on ubuntu and
`windows-latest` from commit 1.

## Decisions still open

Distribution channel and **code signing** — Codex's review argues signing is a ship gate for
milestone 6, not a deferred decision. Maintainer model. The moh-db relationship: worth telling
them, and worth asking for published digests and a `gameType` filter that filters.

## Follow-up, not blocking

The PRD's BSP-checksum bullet describes that work as heavier than it proved to be. Worth
softening once milestone 1 lands — a wording fix.
