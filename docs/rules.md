# Rules

The behavioural contracts Reveille is built to. **Breaking one is a bug, not a style disagreement.**

This file is the register: it holds the rule statements and their identifiers. Other documents
hold the detail that is specific to them — `docs/ui.md` §4 says how the *interface* satisfies a
rule, `docs/engine-facts.md` gives the engine source that *justifies* it. When a rule changes,
change it here first.

## How to read an entry

| Field | Meaning |
|---|---|
| **ID** | Stable. Cite it from code comments, issues, and reviews. Never renumber; retire instead. |
| **Rule** | What must always or never happen. Written as a prohibition where possible — they are easier to check. |
| **Because** | The fact that forces it. A rule with no *because* is a preference, and should be argued rather than obeyed. |
| **Enforced at** | Where in the code the rule is actually made true, and the test that would fail if it stopped being true. |

**A rule whose "Enforced at" says *nothing* is a rule that will be broken.** That column is the
point of this file — it is the difference between a principle and a guarantee.

---

## H — Honesty: what Reveille may claim

### H1 · Never call a client count "players" or "humans"
**Because** `numplayers` is `SV_NumClients()` — every non-free slot, with no way to distinguish a
person from a bot or a parked connection (`engine-facts.md` §5).
**Enforced at** `discovery/model.rs` `ReportedOccupancy`; the type is named to make the mistake
hard. Test: `discovery/client.rs::does_not_infer_bots_from_retail_minplayers`.

### H2 · Bots are a separate, disjoint quantity and are never summed into the client figure
**Because** Bots are not in `svs.clients`, so adding them double-counts. Verified live on all 11
bot servers (`plan.md:158-163`).
**Enforced at** `ReportedOccupancy` owns the two numbers separately; `reveille-cli` renders
`0 clients (+6 bots) · cap 32`. Test: `reveille-cli::renders_more_bots_than_clients_additively`.
**Permanent limit** Whether bots consume human slots is **not observable** — `sv_sharedbots` is
`CVAR_LATCH`, default `0`, and appears in no reply. Do not "fix" this later by guessing.

### H3 · Never emit a boolean "can I join"
**Because** The honest answer has four values: `Compatible` / `NeedsMaps(n)` / `NoSource` /
`CantTell`. `Compatible` means only *nothing checkable is wrong* — the server still decides.
**Enforced at** `join.rs:45` `enum CompatibilityState`; `classify_server` at `join.rs:136`.

### H4 · Never report a moh-db download as verified
**Because** moh-db publishes no hash on any record — only `filename` and `filesize`
(`engine-facts.md` §1). A digest computed after download is *recorded*, not verified: it is
trust-on-first-use.
**Enforced at** The type system keeps them apart: `MohDbIntegrity::RecordedSha256`
(`content/archive.rs:22`) versus `PakRadarIntegrity::VerifiedMd5` (`:30`). No moh-db path can
produce a verified value.

### H5 · Never imply a release digest proves publisher authenticity
**Because** GitHub's per-asset digest arrives from the same origin as the download. It defends
against corruption and a bad CDN edge, **not** against a compromised account.
**Enforced at** Copy only — `ui/views/setup.js` promises the download arrived intact and never
says safe or verified. No test. *This is a copy rule with no mechanical guard; `copy-review` is
the check.*

### H6 · Never state a cause that was not observed
**Because** A release that published no file check was never downloaded, so calling it a
corrupted download is false. Reading the cause out of a formatted error string in the shell is
how those two got merged.
**Enforced at** `reveille-app/src/main.rs` `OpenMohaaFailureKind`, an exhaustive match so a new
error variant fails to compile until classified. Test:
`a_release_without_a_published_file_check_is_not_reported_as_a_bad_download`.

### H7 · Never imply free slots
**Because** `capacity - clients` is not a number Reveille can stand behind — see H2's permanent
limit.
**Enforced at** Capacity appears only as a denominator (`21/32`). The subtraction is never
computed. No test.

### H8 · Say where files actually went
**Because** When an install falls back to `%APPDATA%\moh\main`, a player who later wants to
delete a map must be able to find it.
**Enforced at** `used_home_fallback` is plumbed to the shell (`main.rs:138,163,691,901`) and the
real path is printed, not a euphemism.

### H9 · A failure is a recorded non-result, never an aborted pass
**Because** An unreachable server or a failed catalogue lookup is information about that item,
not a reason to discard the other 112.
**Enforced at** `content/mohdb.rs:148` `non_results`; `discovery/client.rs`. Tests:
`records_missing_and_malformed_hostports_as_non_results`.
**Deliberate exception** C2.

### H10 · Never recommend replacing an installed engine without version evidence
**Because** Finding an engine executable proves presence, not age or provenance. Treating every
present executable as outdated turns a successful install into a permanent update prompt.
**Enforced at** `reveille-app/src/main.rs` validates app-written package receipts against the
installed client files before calling that exact package current. Exact pinned executable hashes
identify the current Reborn package; a historical receipt identifies only a known other build;
anything else is version unknown. The setup view distinguishes current / another known build /
unknown build / absent. Tests cover unchanged and externally changed installed files.

### H11 · Never call the measured round trip the in-game ping
**Because** Three different numbers get the same word. The engine's ping is an average over a
live connection; `sv_minPing`/`sv_maxPing` are the server's admission gate; what Reveille has is
**one** UDP `getstatus` sample taken while fifteen other probes were in flight. It is honest
about distance and dishonest about latency, and a player who reads it as the second will blame
the wrong thing when the game stutters.
**Enforced at** `discovery/model.rs` `RoundTripMillis` is a distinct newtype from `PingMillis`,
so the two cannot be assigned to each other; the field is `Server::status_round_trip`. The Ping
column's tooltip (`ui/lib/format.js` `roundTrip`) says "measured once during this check. Not the
in-game ping." Test:
`discovery/client.rs::keeps_the_measured_round_trip_apart_from_the_servers_own_ping_gate`.
**Never** synthesise a value. A server that produced no reply is not listed at all, so there is
no unknown case to fill in.

### H12 · Never present a remembered server's facts as current, and never call a launch a join
**Because** Two different claims, one cause. A bookmark is written during one check and read
during another: the client count, map and round trip it saw were true of a moment that has
passed, and drawn in the live table they would read as now. And Reveille observes that it
*started the game* — admission is decided at connect time by bans, capacity, a password and the
ping gate (S1), and no reply ever tells Reveille the answer.
**Enforced at** `ui/lib/bookmarks.js` stores an address, a query port and a name and **nothing
else**, so there is no measurement to render as current; a remembered server the current check did
not return is drawn by `absentRow` in `ui/views/servers.js` with its name marked *remembered* and
the words "not in this check", never "offline" and never with figures. History is written only
from `LaunchOutcome::Launched` (`ui/app.js`), never from a refusal, and every label reads
"Launched". No test — this is a storage-shape and copy rule; `copy-review` is the check.
**Not "offline"** A server missing from the sweep was usually never asked: the master returned a
list and only that list was probed. Only a check that actually ran and failed may say the server
did not answer, and it carries the recorded reason.

---

## S — Safety: what Reveille may do to a machine or a server

### S1 · Never send a `connect` packet across a server list
**Because** `getchallenge` proves only that a server is awake — `SV_GetChallenge`
(`sv_client.c:35-110`) issues a token *before* bans, capacity, protocol and ping are tested.
Actually predicting a rejection needs a real `connect`, which on success creates a live client on
someone else's server. That is a join, not a probe.
**Enforced at** Preflight uses `getstatus` only.

### S2 · Never overwrite or activate engine files while one of their programs is running
**Because** Replacing files used by a live game, dedicated server or launcher corrupts an
installation, and on Windows fails part-way through.
**Enforced at** package installation and engine activation require a conservative process query
to confirm that every affected program is stopped. The query is run **after** a download and
before the transactional apply, so a program started mid-transfer is still seen. An unavailable
or malformed process result is unknown and blocks the change. Tests cover the OpenMoHAA release
programs and `MOHAA.exe`, `moh_spearhead.exe`, and `moh_breakthrough.exe`, including case and
malformed output.
**Scope** The probe covers every executable a package replaces, not only the selected client — a
running dedicated server or expansion client can hold another file in the same transaction. The
platform result records which kind was observed so interface copy states only what was known.

### S3 · Never raise a UAC prompt mid-journey
**Because** The ten-minute criterion cannot absorb one, and a player who declines is stranded.
**Enforced at** Writability is *probed*, never inferred from the path string; an unwritable
folder falls back (OpenMoHAA) or is reported as a real blocker (retail, which has no home path).

### S4 · Never let a network call into a default test
**Because** A test that needs a third party is not a test of this code.
**Enforced at** Fixtures frozen under `tests/fixtures/`; live checks are `#[ignore]` and run only
via `just live*`.

### S5 · Never install Reborn before preserving existing retail executables
**Because** Legacy Reborn packages replace the retail executables at their canonical filenames.
Without an immutable first-seen copy, selecting Reborn could make the player's original program
impossible to restore.
**Enforced at** the Reborn activation transaction first copies every existing affected retail
executable to `.reveille-engines/original/` with no-clobber semantics, then records its hash in
`.reveille-engines/state.json`. A later install or activation never replaces that backup. Any
partial activation restores all canonical files. Tests cover first install, reinstall, switching
both directions, externally changed files, and rollback.

---

## C — Content: what Reveille installs

### C1 · Reject downloaded archives containing `.exe` or `.dll`
**Because** A map pack is data. An archive that ships an executable is not what it claims to be.
**Enforced at** `content/archive.rs:107`. Test:
`rejects_windows_traversal_and_executable_entries`.

### C2 · `inspect_archive` rejects the whole archive when any entry fails to parse
**Because** This is the **deliberate opposite** of M1's per-entry skip, and must not be "tidied
up" into consistency. M1 indexes files the player already chose to have; M3 decides whether to
write a stranger's archive into the game directory, where "some entries were fine" is not a
reason to trust the rest.
**Enforced at** `content/archive.rs` `inspect_archive`.

### C3 · Never auto-apply an ambiguous match
**Because** Picking a plausible file for the player is how the wrong map gets installed.
**Enforced at** Choice radios start with nothing selected; the total excludes unresolved maps and
the pane says how many still need a choice.

---

## L — Legal

### L1 · Never redistribute EA assets
**Because** Reveille works against assets the player already owns. Detection and linking to a
store only.
**Enforced at** No asset is ever served or mirrored by this project.

### L2 · Every source file carries `SPDX-License-Identifier: GPL-2.0-only`
**Because** The repository licence is GPL-2.0-only, matching openmohaa.
**Enforced at** `tools/check-sources.mjs`, run by `just sources` and `just check`.

---

## E — Evidence: how Reveille knows things

### E1 · Re-verify a claim against engine source or live measurement before building on it
**Because** Several plan assumptions turned out to be wrong and were caught exactly this way.
**Enforced at** Habit, plus `just engine-source` / `just engine-grep`. No mechanical guard.

### E2 · Put the engine source line beside every protocol constant
**Because** A constant with no citation cannot be re-verified when the engine changes.
**Enforced at** Convention (`AGENTS.md`). Reviewed by hand. No mechanical guard.

### E3 · Record corrections; do not silently apply them
**Because** The reasoning that produced a correction is worth more than the correction. `plan.md`
carries them so the question does not get re-opened from scratch.
**Enforced at** `docs/plan.md`.

---

## Changing a rule

1. Edit the entry here.
2. Check whether `docs/ui.md` §4 or `docs/engine-facts.md` §5 restate it, and update those.
3. If the rule has an "Enforced at" test, change the test in the same commit — a rule and its
   guard should never disagree.
4. If you are *removing* a rule, say why in `plan.md`. Rules here exist because something went
   wrong once.

## Known gaps

H5, H7, H12, C3, E1 and E2 have **no mechanical guard**. They hold only as long as someone is
looking. H5 and H12 are partly covered by the `copy-review` agent; the others are not covered at
all, and that is worth knowing before trusting this list as a safety net.

H12's storage half is stronger than its copy half: `bookmarks.js` never persists a measurement,
so the stale figure a reviewer would look for does not exist to be rendered. What is unguarded is
the wording — "not in this check" versus "offline", "Launched" versus "Joined".
