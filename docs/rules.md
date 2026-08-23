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
**Because** Finding `openmohaa.exe` proves presence, not age. Treating every present executable as
outdated turns a successful install into a permanent update prompt.
**Enforced at** `reveille-app/src/main.rs` validates an app-written release receipt against the
installed client file before calling that exact release current. The setup view distinguishes
current / another known build / unknown build / absent; only another known build is offered a
switch, and reinstalling a current build is secondary. Test:
`a_receipt_only_identifies_the_unchanged_client_and_exact_release`.

---

## S — Safety: what Reveille may do to a machine or a server

### S1 · Never send a `connect` packet across a server list
**Because** `getchallenge` proves only that a server is awake — `SV_GetChallenge`
(`sv_client.c:35-110`) issues a token *before* bans, capacity, protocol and ping are tested.
Actually predicting a rejection needs a real `connect`, which on success creates a live client on
someone else's server. That is a join, not a probe.
**Enforced at** Preflight uses `getstatus` only.

### S2 · Never overwrite release files while one of their programs is running
**Because** Replacing files used by a live game, dedicated server or launcher corrupts an
installation, and on Windows fails part-way through.
**Enforced at** `platform/openmohaa.rs` — `ClientActivity` must be `ConfirmedStopped` before any
replacement; anything else returns `UpdateOutcome::Deferred`. The activity probe is a closure run
**after** the download, not before, so a program started mid-transfer is still seen. Test:
`installs_then_refuses_to_overwrite_while_running_or_unknown`.
**Scope** The probe covers every executable a release archive replaces, not only `openmohaa.exe`
— a running dedicated server holds the same files. The platform result also records whether the
observed process was the game, dedicated server or launcher, so interface copy names only what the
process list established.

### S3 · Never raise a UAC prompt mid-journey
**Because** The ten-minute criterion cannot absorb one, and a player who declines is stranded.
**Enforced at** Writability is *probed*, never inferred from the path string; an unwritable
folder falls back (OpenMoHAA) or is reported as a real blocker (retail, which has no home path).

### S4 · Never let a network call into a default test
**Because** A test that needs a third party is not a test of this code.
**Enforced at** Fixtures frozen under `tests/fixtures/`; live checks are `#[ignore]` and run only
via `just live*`.

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

H5, H7, C3, E1 and E2 have **no mechanical guard**. They hold only as long as someone is
looking. H5 is partly covered by the `copy-review` agent; the others are not covered at all, and
that is worth knowing before trusting this list as a safety net.
