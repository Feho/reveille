<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille — implementation supplement

Companion to [`plan.md`](plan.md). That plan says *what* and *why*; this says
*exactly what the values are*, so the work can be checked against reality instead of against
itself. **Self-contained by design** — every constant the plan cites by file path is inlined
here, because the citations point outside the new repo.

---

## 0. Access

The plan cites two trees that are **not** inside `/home/feho/dev/reveille`:

| Path | Contains | Needed for |
|---|---|---|
| `/home/feho/dev/openmohaa/code/` | engine source — ground truth | verifying protocol claims |
| `/home/feho/MOHAA/` | live server, retail pk3 corpus, Python spikes | reference impls + the test fixture |

If both are readable, use them — they are authoritative and this document is a summary.
**If they are not readable, this document is sufficient**; nothing below requires them.

Reference implementations, if reachable: `tools/spike_fingerprint.py` (map index),
`spike_masterlist.py` (GameSpy handshake), `spike_pipeline.py` (full discovery chain),
`spike_connect.py` (out-of-band probe + rejection table).

---

## 1. Constants

### GameSpy master (`sv_gamespy.c:42,50`)

```
master host/port : master.333networks.com : 28900   (TCP, GameSpy v1)
gamenames        : ["mohaa", "mohaas", "mohaab"]           // AA, Spearhead, Breakthrough
secret keys      : ["M5Fdwc", "h2P1c9", "y32FDc"]          // index-aligned with gamenames
```

Handshake: connect → read `\secure\<challenge>\` → reply

```
\gamename\{g}\gamever\1\location\0\validate\{V}\final\\queryid\1.1\list\cmp\gamename\{g}\final\
```

where `V = gs_encode(gs_encrypt(key, challenge))`. Response body is 6 bytes per server —
4-byte IPv4 then big-endian u16 **query** port — terminated by `\final\`.

`gs_encrypt` is an RC4 variant; `gs_encode` is base64 over zero-padding to a multiple of 3
with `=` stripped. Both are ported in `spike_pipeline.py:45-64`; port them from there rather
than reinventing, and unit-test against a captured challenge/validate pair.

> The query port from the master is **not** the game port. Read `hostport` from the GameSpy
> `\status\` reply. It is typically 12203 but 23900 has been observed. Never compute an offset.

### Out-of-band header

```
client -> server : ff ff ff ff 02
server -> client : ff ff ff ff 01
```

Five bytes, **not** Quake 3's four. Sending the plain Q3 header gets silence, which is
indistinguishable from a firewalled port. Commands: `getstatus`, `getinfo <challenge>`,
`getchallenge`.

### BSP header

```
offset 0  : ident    char[4]  "2015" (AA) or "EALA" (expansion) — INFORMATIONAL, never gate on it
offset 4  : version  i32 LE   19 across the corpus — GATE ON THIS, valid range 17..=21
offset 8  : checksum i32 LE   <-- this is sv_mapChecksum
```

**Which field is authoritative.** `CM_LoadMap` (`cm_load.c:894`) never reads `ident` at all; its
only check is `version` against `BSP_MIN_VERSION` 17 / `BSP_MAX_VERSION` 21 (`qfiles.h:361-364`).
Rejecting an unknown ident hides a map the engine would happily load; accepting an out-of-range
version reports as present a map that will `ERR_DROP` on join. Corpus distribution: 70 × `2015`
v19, 18 × `EALA` v19, none outside 17..=21.

`CM_Checksum` (`cm_load.c:752-771`) has its lump-hashing path commented out and returns
`header->checksum` verbatim; `CM_LoadMap` uses it at `cm_load.c:887`. **Read 12 bytes; do not
parse lumps.** Note the value is signed and frequently negative on the wire.

### Rejection strings (`sv_client.c`, verbatim)

```
droperror\nYou are banned from this server.\nReason: %s
droperror\nYou are banned from this server.
droperror\nServer uses protocol version %i
droperror\nServer is full
droperror\nKicked from server for:\n%s
droperror\nKicked from server
droperror\nRequires Medal of Honor Allied Assault Breakthrough
droperror\nUserinfo string length exceeded.
print\nNo or bad challenge for your address.
print\nServer is for high pings only
print\nAwaiting CD key authorization
```

### moh-db

```
base : https://api.moh-db.com/api/external/v1/{maps,mods}
```

- **Requires a real `User-Agent`.** The default Python urllib UA gets `403 Forbidden`; curl
  succeeds. Set an explicit UA identifying Reveille.
- Spring-style paging: `?size=&page=&mapName=`; response has `content[]` + `totalElements`.
- **`gameType` is accepted and ignored** — every value returns the same 2,528 records. Filter locally.
- **No integrity hash on any record.** Only `filename` and `filesize`. Downloads are
  trust-on-first-use: record the digest, never call it verified.
- `mapName` values carry stray whitespace and inconsistent case — see normalisation below.

### Reborn legacy player packages

The supported Windows player packages come from `mohreborn/mohreborn-docs` commit
`15451e40274e718870dcf8ba295bb8fcde745857`. The repository's player instructions select a ZIP
by installed products and replace the canonical retail executable names. Reveille implements the
same four-way mapping but preserves originals first:

| Data directories | Package | Bytes | SHA-256 |
|---|---:|---:|---|
| `main` | `mohreborn_aa.zip` | 733,577 | `e38a41810a81e40239245c57d549ee19250f84e46595c1d93d1cddea71d6f333` |
| `main` + `mainta` | `mohreborn_aa_sh.zip` | 1,537,186 | `fc586d1739fc390709bf07ea9237ae02a24aab84f504a295b5975d0cbc349a45` |
| `main` + `maintt` | `mohreborn_aa_bt.zip` | 1,553,694 | `7ac402f4d74893c4df06c6d418162812cbfb3060ce353e914bee2d21908e9dc0` |
| all three | `mohreborn_aa_sh_bt.zip` | 2,357,315 | `425cbe3a4253f62b9f088c7715d393b17b929b56631f230ebd99de88d45be457` |

The archives contain only `MOHAA.exe`, `moh_spearhead.exe`, and/or
`moh_breakthrough.exe` beneath one package directory. Exact executable hashes are frozen in
`platform/reborn.rs`. The newer `mohreborn/releases` repository had **zero published releases on
23 Aug 2026**; its future signed side-by-side format is intentionally outside this change.

---

## 2. Map-name normalisation (exact)

Applied identically to index keys, `sv_maplist` entries, and moh-db `mapName` values:

1. trim leading/trailing whitespace — moh-db really returns `"obj/obj_howitzer "` with a
   trailing space, and matching fails without this
2. replace `\` with `/`
3. lowercase (ASCII)
4. strip a leading `maps/` and a trailing `.bsp`
5. **nothing else** — no prefix insertion, no de-duplication of slashes, no stemming

Keep the original spelling alongside the key for display.

**Both prefixed and bare names are legitimate.** 37 of the fixture's 88 keys have no `/` at all
(`m6l2a`), and one live `sv_maplist` mixes `DM/mohdm6` with `dm/mohdm7` in the same string.
A normaliser that assumes a `dm/`-or-`obj/` prefix will silently drop 42% of the index.

---

## 3. Search-path precedence (exact)

From `files.cpp:3106-3240`. `fs_searchpaths` is a **prepend-to-head** linked list walked from
the head, so **last added wins**. Within one game directory the order of addition is:

1. `.pk3` files and `.pk3dir` directories, interleaved, **alphabetically**
2. **the plain game directory itself, last**

Resolving that to lookup order (highest precedence first):

```
1. loose files in main/            <- beats everything
2. pk3 / pk3dir, REVERSE alphabetical   (zzzz_ before Pak0 — this is why the prefix works)
```

Lookups are case- **and** separator-insensitive (`FS_FilenameCompare`, `files.cpp:1362`).
Multiple base paths (basepath, then homepath) are each added in full, so homepath wins.

The index must return **all** providers in this order, not collapse to one.

---

## 4. Frozen fixtures

Commit these as static files under `crates/reveille-core/tests/fixtures/`. **Do not hit the
network in tests** — the servers below are third-party and will change or vanish.

### 4a. `install_scan.json` — expectations for `/home/feho/MOHAA`

```json
{
  "note": "This path is a DEDICATED SERVER, not a client. No mohaa.exe / openmohaa binary exists anywhere on this machine.",
  "pk3_count": 30,
  "expansions": ["main"],
  "maps_indexed": 88,
  "maps_multi_provider": 0,
  "keys_without_slash": 37,
  "loose_bsp_files": 0,
  "pk3dir_count": 0,
  "known_checksums": { "dm/mohdm6": 1974169620, "dm/mohdm7": 391868696 },
  "checksum_source_pak": "Pak5.pk3",
  "awkward_filenames": ["z_Kmarzo-St Renan.pk3"]
}
```

> **Coverage warning — read this before trusting the numbers.** An earlier draft of this file
> claimed `54` multi-provider maps. That was wrong: it came from a **recursive** scan that swept
> up `.claude/worktrees/*/main` and `main/disabled_mods` copies of the same archives. The engine
> is not recursive — `FS_AddGameDirectory` (`files.cpp:3136`) lists `*.pk3` with
> `wantSubs = qfalse` — so an engine-scoped scan sees 30 archives and **no map shadowing at
> all**. This fixture also holds zero loose `.bsp` and zero `.pk3dir` (there is a `main/maps/`
> directory, but it is empty of maps). **Reproducing these numbers therefore proves nothing about
> precedence or the loose-file path.** Synthetic `tempfile` fixtures are their *only* coverage —
> build them deliberately, covering loose-beats-pk3, `.pk3dir`, and a `zzzz_` pk3 beating an
> alphabetically earlier one.

### 4b. `server_tfc.json` — a real server, captured 19 Aug 2026

`<[TFC]> Sniper Only OBJ | www.tfc-clan.com` at `173.249.214.104:12203`.

```json
{
  "protocol": "8",
  "sv_hostname": "<[TFC]> Sniper Only OBJ | www.tfc-clan.com",
  "sv_info": "MoH:AA 1.12 Reborn Patch RC3.5 (NIX)",
  "version": "Medal of Honor Allied Assault 1.11 linux-i386 Jul 22 2004",
  "mapname": "obj/blutstein",
  "g_gametypestring": "Objective-Match",
  "sv_allowDownload": "0",
  "sv_maxclients": "32",
  "sv_privateClients": "4",
  "g_allowjointime": "10",
  "sv_minPing": "0",
  "sv_maxPing": "0",
  "sv_maplist": "dm/mohdm2 obj/stlo dm/mohdm3 obj/obj_fallenvillage obj/blutstein dm/mohdm7 obj/questufou_s_yvette_obj obj/obj_morning2 obj/obj_dessau1945fix obj/obj_rush_party dm/mohdm6 obj/obj_howitzer obj/thechurch_final_obj obj/renan"
}
```

Note `sv_mapChecksum` and `pure` are **both absent** — this server publishes neither. That is
the majority case (`sv_mapChecksum` on 30/127, `pure` on **0/127**) and must be a first-class
code path, not an error. Note also `sv_info` says 1.12 while `version` says 1.11.

Expected preflight against fixture 4a: **7 present, 7 absent.**

```
present : dm/mohdm2  obj/stlo  dm/mohdm3  obj/blutstein  dm/mohdm7  dm/mohdm6  obj/renan
absent  : obj/obj_fallenvillage  obj/questufou_s_yvette_obj  obj/obj_morning2
          obj/obj_dessau1945fix  obj/obj_rush_party  obj/obj_howitzer  obj/thechurch_final_obj
```

### 4c. `mohdb_resolution.json` — all three outcomes from one real rotation

| Wanted (`sv_maplist`) | moh-db hits | Outcome | Resolves to |
|---|---|---|---|
| `obj/obj_fallenvillage` | 1 | **exact** | `User-obj_FallenVillage.pk3` · 2.4 MB |
| `obj/obj_howitzer` | 1 | **exact** *(only after trimming the trailing space in the catalogue's `mapName`)* | `obj_howitzer_v1_1.pk3` · 1.5 MB |
| `obj/thechurch_final_obj` | 2 | **exact**, 1 of 2 | `Obj_TheChurch_Final.pk3` · 3.0 MB |
| `obj/obj_dessau1945fix` | 2 | **exact**, 1 of 2 — the other is `obj/obj_dessau1945` without the fix | `obj_dessau1945fix.pk3` · 2.1 MB |
| `obj/questufou_s_yvette_obj` | 1 | **near match — needs a choice** | catalogued as `dm/Questufou_s_Yvette`, file `Questufou_S_Yvette_Final.pk3` · 4.9 MB. Different prefix *and* suffix |
| `obj/obj_rush_party` | 1 | **near match — needs a choice** | catalogued as `dm/rush_party`, file `rush_party.pk3` · 9.8 MB |
| `obj/obj_morning2` | 0 | **no source** | nothing across 2,528 catalogued maps |

Exact matches total **9.0 MB**; with both near matches accepted, **23.7 MB**. One live server
exercises exact-match, ambiguity, and dead-end — build the resolver's tests on it.

---

## 5. Prohibitions

The rule statements now live in [`docs/rules.md`](rules.md), with an identifier each and the
place in the code where every one is enforced. Change a rule there first. What follows is the
*engine evidence* for the rules that come from this document — the reason each holds, which is
the part that belongs here.

- (S1) **Never send a `connect` packet across a server list.** `getchallenge` is safe and answers
  freely, but it proves only that the server is awake — `SV_GetChallenge` (`sv_client.c:35-110`)
  issues a token *before* bans, capacity, protocol and ping are tested in `SV_DirectConnect`
  (`sv_client.c:389-640`). Actually predicting a rejection needs a real Huffman-compressed
  `connect`, which on success creates a live `CS_CONNECTED` client on someone else's server.
  That is a join, not a probe. Preflight uses `getstatus` only.
- (H1) **Never label a client count "players" or "humans."** `numplayers` is `SV_NumClients()` —
  every non-free slot, with no way to distinguish a person from a bot or a parked connection.
  The type should be named to make the mistake hard.
- (H4) **Never report a moh-db download as verified.** No hash is published; the digest is *recorded*.
- (H3) **Never emit a boolean "can I join".** The states are `Compatible` / `NeedsMaps(n)` /
  `NoSource` / `CantTell`, and `Compatible` means *nothing checkable is wrong*.
- (L1) **Never redistribute EA assets.** Detection and linking to a store only.
- (C1) Reject downloaded archives containing `.exe` or `.dll`. A map pack is data.

---

## 6. Repo conventions to establish in commit 1

The new repo has no conventions file, and openmohaa's `CLAUDE.md` is C++ rules that do not
apply. Seed `AGENTS.md` in `/home/feho/dev/reveille` covering at least:

- `cargo fmt` default; `cargo clippy -- -D warnings` must pass
- errors via `thiserror`; no `unwrap`/`expect` outside tests and `main`
- `reveille-core` is **pure logic** — no `println!`, no process spawning, no exit codes; all
  I/O policy belongs to the callers (`reveille-cli`, `reveille-app`)
- newtypes over bare primitives where a mistake would be silent — map keys, checksums, client
  counts, ports
- every protocol constant carries a source comment (`// sv_gamespy.c:42`) so it can be re-verified
- SPDX `GPL-2.0-only` header on new files; `LICENSE` at the root
