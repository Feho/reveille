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

### H1 · Never call a client count "humans", and never fold bots into it
**Amended 27 Aug 2026.** The rule read *never call a client count "players" or "humans"*, on the
grounds that `numplayers` gives "no way to distinguish a person from a bot or a parked connection"
(`engine-facts.md` §5). Half of that turned out to be wrong, and the project's own measurement is
what shows it: bots are **not** in `svs.clients`, verified live on all 11 bot servers found in the
20 Aug sweep (`plan.md`, milestone 2) — which is exactly why H2 exists to keep them disjoint. A
figure that provably excludes bots is not one that may not be called `Players`; forbidding the word
was over-correction, and it cost every newcomer the one column heading they already understood from
every other server browser they have used.

What survives is the half that is still true. A slot is occupied from `connect` onwards, so it
counts while its holder is downloading or sitting at a menu, and it counts a stale connection the
server has not yet timed out. So the figure is a count of **connections the server counts as
players**, not of people at play, and the two claims Reveille still may not make are:

- **"humans"**, or any wording that promises a person at a keyboard;
- **any sum of clients and bots**, which is H2 and is unchanged.

**Because** the qualifier belongs where a reader can reach it, not in the heading: a column heading
that hedges its own noun is read as a warning about the *number*, when what is uncertain is the
*word*. So the heading is one word and the glossary carries the sentence.
**Enforced at** `discovery/model.rs` `ReportedOccupancy` keeps the two quantities separate and is
named for the wire field, not for the label; `ui/lib/glossary.js` defines **Players** and **Bots**
in reachable text; the status bar's tooltip states the connecting-and-downloading case.
Test: `discovery/client.rs::does_not_infer_bots_from_retail_minplayers`.
*The label itself has no mechanical guard — `copy-review` is the check.*

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
**Because** When an install falls back to `%APPDATA%\openmohaa\main`, a player who later wants
to delete a map must be able to find it.
**Enforced at** `used_home_fallback` is plumbed to the shell (`main.rs:138,163,691,901`) and the
real path is printed, not a euphemism.
**Resolve the destination once per join** `resolve_install_target` *probes*, so it can answer
differently a second time — a folder locked when the preview ran may be writable when the install
finishes. `install_and_launch` therefore reports the preview's destination, the one the files were
actually written to, and re-indexes around it (`install_destination` / `reindex`) rather than
resolving it again.

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
OpenMoHAA package identity is the exact version, asset name and digest, independent of the
selected channel. Legacy `dev` receipts deserialize as `preview` and retain their recorded
version. Tests: `identical_openmohaa_packages_are_current_across_channels` and
`legacy_dev_receipts_keep_their_installed_build_identity`.
**Evidence is not a reason to withhold the action.** Version evidence decides what the button is
*called*, never whether one exists: a build that is not the offered package can always be
replaced, and a card that states a version while offering no way to change it sends the player to
**Continue**, which records the engine choice and installs nothing (`friction.md` F2, observed
5 Sep 2026). `OpenMohaaInstalledBuild::KnownOther` therefore carries an `OfferRelation` decided by
semver precedence in Rust — `newer`, `older`, `same_version`, or `incomparable` for a receipt tag
that predates semver — and the shell names the action from it: **Update to**, **Go back to**,
**Reinstall**, or a plain **Install**. A channel switch offering a lower version may not be called
an update. Tests: `the_offered_release_is_ordered_against_the_installed_one` and
`an_installed_engine_can_still_be_changed_from_setup`, the second a text check over
`ui/views/setup.js` for the same reason as those under H12.

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
not return is drawn by `absentRow` in `ui/views/servers.js` with its name in the *remembered* style
and the words "not in this list" across every column a figure would have been in, never "offline"
and never with figures. The row also carried an explicit "remembered name" line for a while; it was
dropped as redundant, and the rule holds without it because the row says outright that this check
did not return the server, and the name is the only remembered thing left on it. History is written only
from `LaunchOutcome::Launched` (`ui/app.js`), never from a refusal, and every label reads
"Launched". The storage-shape and copy halves have no test; `copy-review` is the check. The two
clauses below are guarded, each by one text check over the shell.
**Not "offline"** A server missing from the sweep was usually never asked: the master returned a
list and only that list was probed. Only a check that actually ran and failed may say the server
did not answer, and it carries the recorded reason.
**Nor a row a later check found gone** A live row's figures were measured once and the pane says
when, so their age is readable rather than assumed: **Checked at 14:32** for a row asked again on
its own, **From the check at 14:32** for one the sweep returned, because a sweep's finish time is
not when any particular row inside it answered. **Check again** re-asks that one server; when it
gets no answer, the row is dropped from the list rather than left standing, because the check that
just ran is evidence about now and the figures it replaces have been shown not to be. The selection
survives so the pane can say what the check found and offer to ask again. A check that could not
*run* is not a server that did not answer, and says so separately — the last measured figures are
still the last measured figures. Enforced at `ui/app.js` `check`, which filters the checked address
out of `state.servers` on a non-result and records `state.checkedAt` on an answer. Test:
`reveille-app::a_check_that_got_no_answer_drops_the_row_it_was_checking` — a text check over
`app.js`, for the same reason as the one below.
**Nor a list swept for another session** The table is the answer to one question — this folder,
this engine, this game — and nothing on it says which. Re-entering setup can change all three, so
`state.listSession` records what the rows were swept for and `enterServers` (`ui/app.js`) sweeps
again whenever it no longer matches, exactly as the toolbar's game switch does. Leaving Spearhead's
servers on screen under Allied Assault would be the same false currency as a bookmark's old figures:
those servers were never asked this question, and their compatibility was judged against a different
search path. Test:
`reveille-app::the_shell_sweeps_again_when_the_session_the_list_was_swept_for_changed` — a text
check over `app.js` and `store.js`, because the shell has no test runner; it guards the exact
regression that shipped, not the behaviour in general.

### H13 · Never index an expansion's directory without the base game underneath it
**Because** Spearhead and Breakthrough do not replace `main`; the engine adds their directory
*after* it (`engine-facts.md` §3a). An index built from `mainta` alone would report every
`main` map on a Spearhead server as missing — a false absence, and one that would send a player
to download files they already own.
**Enforced at** `join.rs` `LaunchProfile::search_directories` gives the chain, lowest precedence
first; `platform::content_search_path` resolves it against the installation and the home path;
`MapIndex::scan_chain` indexes all of it and keeps every provider, so a shadowed copy is still
listed under the one the engine loads. Tests:
`mapindex.rs::an_expansion_directory_shadows_main_without_hiding_it`,
`mapindex.rs::every_scan_count_accumulates_across_the_chain`,
`join.rs::an_expansion_reads_main_underneath_its_own_directory`,
`reveille-platform::an_expansion_search_path_keeps_main_underneath_it`,
`reveille-platform::the_home_copy_of_a_directory_outranks_the_installed_one`.
**Known limit** `content_search_path` models the *selected* installation and the home path, not
`fs_steampath` / `fs_gogpath` / `fs_microsoftstorepath` or a non-empty `fs_game`
(`files.cpp:3562-3573,3647-3650`). A map present only in a second, unselected installation, or
only in a server-published mod directory, is loadable by the engine and reported missing here.
Modelling the other roots means deciding which installation the player meant, which is the
question setup already asked them.
**Not symmetrical** Breakthrough reads `maintt` and `main`, never `mainta`: `fs_basegame` holds
one directory. Do not "tidy" the three chains into a cumulative one.

### H14 · Never offer a game the installation has no files for
**Because** The three products are sold and installed separately. Browsing Spearhead against an
install with no `mainta` is not a degraded session, it is a false one: every server's rotation
would read as unavailable and no client executable exists to launch.
**Enforced at** `install.rs` `Installation::provides`, and `Installation::playable` — the
*runnable* subset, which is not `products`: an expansion needs the base game underneath it, so a
folder with `mainta` and no `main` provides nothing (H13). `installed_maps` in
`reveille-app/src/main.rs` and `playable_install` in `reveille-cli/src/main.rs` both refuse before
any directory is probed or any home fallback is created, and the toolbar's game switch and setup's
game question are built from `playable`, so an unrunnable game is never offered. `launch_client`
(`reveille-platform`) additionally names the missing program when a spawn fails with `NotFound`.
Tests: `reveille-app::a_game_the_folder_has_no_files_for_is_refused_before_anything_is_probed`,
`install.rs::an_expansion_directory_alone_does_not_make_that_expansion_playable`,
`reveille-platform::an_install_without_the_expansion_client_says_which_program_is_missing`.
**Never pre-check the program as a path** `Command` resolves a bare name against `PATH` and
`Path::is_file` does not, so a pre-spawn existence check refuses a client that is installed — the
CLI's default join client is the bare name `openmohaa`. Classify *after* the spawn attempt. Test:
`reveille-platform::a_bare_program_name_is_resolved_rather_than_treated_as_a_path`.

### H15 · Never fold a remembered entry away without stating how many are folded
**Because** Favorites and History are collapsed by default down to what the current check
returned, because a folder with more than one game fills them with entries that can never answer:
the three register with the master separately, so a server saved under Spearhead is not in Allied
Assault's list and its **Check** can only ever find the same thing. Twenty rows of "not in this
check" above three that answered reads as a broken list. But a fold that does not say what it is
folding is a filter with an invisible effect, which is exactly what got **Hide unavailable maps**
removed (`ui.md` §2.1) — the player cannot judge a control whose result is off screen.
**Nor may it classify** Reveille does not know which game a saved address runs. A bookmark is an
address (H12), and the game it was saved under is a fact about a past moment; folding on a stored
or re-stamped "last seen under Spearhead" would hide a server that has since moved games under the
game it no longer runs. The criterion is the one the rows themselves already state and that this
check demonstrably established: **it was not in this list**.
**Enforced at** `ui/lib/store.js` `scopedRows` emits the `disclosure` item whenever the absent set
is non-empty — open or shut, so the count is on screen either way — and only then omits the rows
behind it. `ui/views/servers.js` `disclosureRow` draws it with `aria-expanded`, in the row where
those entries would have been, and `liveText` repeats the folded count for a screen reader.
`scopedStatusbar` withholds **Check the other N** while the block is shut, because the whole of
that button's effect is inside the block. `ui/app.js` `autoCheckFavorites` waits for the same
thing, for the same reason. Test:
`reveille-app::folded_remembered_entries_always_state_their_count` — a text check over `store.js`
and `servers.js`, for the same reason as the ones under H12: the shell has no test runner and the
failure would be silent.

### H16 · Never offer the installed or an older Reveille release as an update
**Because** A GitHub release title or installer filename is not version evidence. Re-offering the
installed version would turn every launch into a permanent update prompt, and offering an older
one would silently turn update into rollback.
**Enforced at** `reveille-app/src/self_update.rs` delegates semantic-version comparison of the
latest published manifest to Tauri's updater. The pending update is the exact checked offer,
retained in Rust rather than reconstructed from frontend fields. `ui/app.js` exposes **Update
Reveille** only when that check returns a newer version, and installation starts only from the
player's **Update and restart** action. Payload authenticity is the separate, stronger install
gate in S6: the release label is not what the updater signature covers. Test:
`reveille-app::the_self_update_offer_is_explicit_and_keeps_the_checked_release`.

### H17 · The engine channel is decided by semver precedence, never by publication order
**Because** OpenMoHAA engine releases now carry immutable semver tags rather than a rolling `dev`
tag. GitHub returns `/releases` in creation order, which puts a hotfix cut from an older branch
ahead of a newer release candidate; taking the first entry would silently offer a player a *lower*
version than the one they have. A tag is also not a channel: a release counts as a prerelease when
its tag says so **or** GitHub's `prerelease` flag says so, so a publishing mistake in either place
cannot leak an untested build into the stable channel.
**Enforced at** `reveille-core/src/platform/openmohaa.rs` — `ReleaseVersion` implements semver
precedence (§11.4), `parse_release_list` selects by `max_by` on the parsed version, drafts are
excluded from both channels, and a non-semver tag is skipped rather than failing the channel for
every player. The preview channel deliberately admits stable releases so a player on it is offered
`v0.83.0` once it outranks `v0.83.0-rc.2` rather than being stranded on the candidate. Tests:
`preview_channel_takes_the_highest_semver_release_not_the_newest_entry`,
`stable_selection_from_a_list_skips_every_prerelease_and_draft`,
`preview_channel_offers_a_stable_release_once_it_outranks_the_candidate`,
`a_stable_tag_flagged_prerelease_stays_out_of_the_stable_channel`,
`a_non_semver_tag_is_skipped_rather_than_failing_the_whole_channel`,
`semver_precedence_orders_candidates_below_their_release`.
The single-release path applies the same draft and prerelease guard before asset selection.
Full tags, including prerelease identifiers and build metadata, are validated by `semver`.
Preview fetches every release-list page before selecting the maximum; a higher version outside
the first page must not be lost, and a failed later page fails the check rather than offering
a potentially older build. Test: `preview_fetches_every_page_before_selecting_a_release`.
Tests: `single_release_selection_enforces_channel_and_draft_guards` and
`invalid_semver_identifiers_cannot_win_release_selection`.

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

### S6 · Never replace Reveille from an unsigned release or without the player's choice
**Because** A self-update replaces the program currently making trust decisions on the player's
behalf. A background network response is not consent, and transport security alone does not bind
downloaded bytes to the release key embedded in the installed app.
**Enforced at** release builds require Tauri's private updater key and publish its detached
signature inside `latest.json`; the app is compiled with the corresponding public key. Tauri
refuses an invalid signature before `Update::install`, and Windows exits the running app before
NSIS replaces it. `install_reveille_update` is reachable only through the visible **Update and
restart** button; **Later** closes the offer without installing. The download can be stopped before
the verified apply phase. Test:
`reveille-app::the_self_update_offer_is_explicit_and_keeps_the_checked_release`.

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

H12 is **partly** guarded, and the split matters. Two of its clauses have a text check each — the
list swept for another session, and the row a later check found gone. Its storage half needs none:
`bookmarks.js` never persists a measurement, so the stale figure a reviewer would look for does not
exist to be rendered. What is unguarded is the wording — "not in this list" versus "offline",
"Launched" versus "Joined" — and `copy-review` is the check for that. Both text checks guard the
exact regression that shipped, not the behaviour in general: neither would catch the same rule
broken in a new place.
