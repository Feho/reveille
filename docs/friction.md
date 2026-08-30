# Friction ledger

Reveille exists to remove the things that stop a player getting into a game. This file is the
list of those things. It is the product spine: **an issue should trace back to an entry here**,
and closing the issue updates the entry rather than the plan.

## Why this is separate from `plan.md`

`docs/plan.md` records *what we decided and what we measured*. This file records *what hurts a
player*. They change for different reasons and at different rates — a measurement gets corrected
once, whereas an entry here changes every time you learn something about real players.

## The rule that keeps it honest

The same discipline the code follows applies to the product: **say how you know.** Every entry
carries an evidence line, and it is one of three words.

- **Measured** — a number from a live sweep, the engine source, or a fixture, with a date. Cite
  it the way the code cites engine constants.
- **Observed** — someone actually hit this. A forum thread, a Discord message, the owner's own
  machine. Say who and where.
- **Assumed** — believed, not established. **Assumed entries may not be ranked above measured
  ones**, and an assumed entry that has stayed assumed for two milestones is a sign it should be
  cheaply measured or dropped.

An entry with no evidence line is not an entry. It is a feature idea, and it belongs in an issue,
not here.

## Order

Roughly by journey stage, not by priority — priority comes from the "who it stops" figures and
changes too often to encode in a heading.

---

## Stage 1 — Getting a working client

### F11 · Windows warns about Reveille before it ever runs

**Where** The download, and the first run of the installer. Before setup — this is the step ahead
of F1.
**Who it stops** Unknown. Everyone cautious enough to stop at "Windows protected your PC", which
for a launcher aimed at people who have not played since 2002 is not a small share.
**Evidence** *Assumed.* Nothing has been measured, because nothing has been distributed. What is
established is the mechanism: a download from GitHub Releases carries Mark of the Web, an unsigned
installer names no publisher in the UAC prompt, and SmartScreen reputation accrues per signing
certificate — so an unsigned build starts at zero and stays there.
**What Reveille does** Nothing yet. The installer is per-user, so at least it raises no
Administrator prompt on top of the warning, and the release notes and README say plainly that the
build is unsigned rather than letting the warning be a surprise.
**Status** Open. Packaging shipped 27 Aug 2026; signing is decided (SignPath Foundation) and not
yet applied for, which is deliberate — their conditions require the project to be released in the
form to be signed. See `plan.md`, *Windows packaging and signing*. **This entry should stop being
Assumed:** the first release makes it cheaply Observed.

### F12 · An installed Reveille never learns that a fix was published

**Where** Every launch after the first release.
**Who it stops** Anyone whose problem was fixed in a newer build but who does not revisit the
GitHub release page on their own.
**Evidence** *Observed, 28 Aug 2026.* Requested directly by the project owner. No player frequency
has been measured yet.
**What Reveille does** Checks GitHub's latest published release in the background. When its
published semantic version is newer, Reveille offers **Update Reveille** with **Update and restart** and
**Later** choices. A failed check does not interrupt setup or browsing; the download is cancellable,
and no bytes are installed until Tauri verifies the release signature embedded by the release
workflow.
**Status** Shipped. The owner must create the updater key once and store its private half in GitHub
Actions before the next release; the public half is compiled into release builds.

### F1 · The player cannot tell Reveille where the game is

**Where** Setup, first run.
**Who it stops** Unknown. Every non-standard install, plus every store whose layout we have not
seen.
**Evidence** *Assumed.* Discovery covers registry `Uninstall` keys, GOG Galaxy manifests, EA App
/ Origin roots, and common literal paths (`plan.md`, M5), but **no measurement exists** of how
often all of those miss. The manual folder picker is the safety net and its hit rate is unknown.
**What Reveille does** Auto-detects, then falls back to a folder picker, and says how confidently
it identified the install — a verified binary hash versus a name-only match.
**Status** Shipped in M5. The measurement is the gap, not the feature.

### F2 · Choosing, installing, or switching the game program

**Where** Setup.
**Who it stops** Anyone who does not already know the difference between the original game,
OpenMoHAA, and Reborn, or how to install and safely switch between them.
**Evidence** *Measured.* Only **11 of 111** live servers report the OpenMoHAA engine string
(`plan.md:163`), so most servers still accept a retail client — but the engine is where fixes for
modern Windows land. The install itself was previously a manual download-and-unzip.
**What Reveille does** Presents three neutral, plain-language choices. OpenMoHAA uses its current
release provider. Reborn uses the official legacy player archive matching the detected data
directories, pinned to an immutable repository commit. Reveille preserves first-seen original
executables before Reborn activation, retains both managed copies, checks running programs, and
remembers the active choice per canonical game folder.
**Status** Shipped for Windows. Installed versions are identified only from exact package hashes
or a receipt that still matches the files; unmatched files remain **Version unknown**.

### F3 · The game folder is not writable

**Where** Setup, and again at every content install.
**Who it stops** Anyone whose game sits under `C:\Program Files (x86)`.
**Evidence** *Assumed.* The case is real and the engine behaviour is measured — the home path
outranks the install directory in the search path (`files.cpp:3245-3257`) — but **how many
installs land in Program Files is not measured.** GOG's standalone installer defaults to
`C:\GOG Games\…`, which needs no elevation, so this may be rarer than it feels.
**What Reveille does** Probes writability rather than inferring it from the path string, falls
back to `%APPDATA%\openmohaa\<game directory>` on OpenMoHAA, and prints the real destination. Never raises a UAC
prompt mid-journey. On retail there is no home path, so an unwritable folder is reported as a
real blocker instead of being worked around.
**Status** Shipped.

---

## Stage 2 — Finding somewhere to play

### F4 · The server list does not say whether you can actually join

**Where** Browse.
**Who it stops** Everyone, on every server, every time.
**Evidence** *Measured, 19 Aug 2026.* On one frozen 114-server set: an 88-map install yields
**84 Compatible / 15 Needs maps / 15 Can't tell**; a stock Pak0–Pak5 retail install (54 maps)
yields **83 / 16 / 15**. Exactly one server flips between them
(`-<MisFits>- Rifle/sniper`). An independent Python reimplementation agreed with the Rust
classifier (`plan.md:251`).
**What Reveille does** Four states, never a tick, and `Compatible` is explained as the limit of
what can be checked — the server still decides whether you get in.
**Status** Shipped.
**What this number actually says** A newcomer with nothing beyond retail already reaches **~73%**
of the live population. Content resolution is the last mile for ~14% of servers, **not** the
primary blocker. Resist ranking F5 above F4 on instinct; the measurement says otherwise.

### F5 · The server list overstates how busy a server is

**Where** Browse.
**Who it stops** Anyone choosing between servers — a wrong count sends you to an empty game.
**Evidence** *Measured.* Bots are not in `svs.clients`, so a naive reading double-counts them.
Verified live on all **11** bot servers (`plan.md:158-163`).
**What Reveille does** Renders `0 clients (+6 bots) · cap 32` — never summed, never merged.
Sorts on reported human clients only.
**Status** Shipped.
**Known unsolvable** Whether bots consume human slots is **not observable**: `sv_sharedbots`
(`CVAR_LATCH`, default `0`) appears in **no** reply, 0 occurrences across the corpus
(`plan.md:168-171`). This is a permanent limit, not a to-do. Do not let anyone "fix" it later by
guessing.

### F9 · No way to get back to a server you liked

**Where** Browse, on the second and every later run.
**Who it stops** Anyone who has already found a server worth returning to — so, by definition,
nobody on their first run and potentially everybody after it.
**Evidence** *Assumed.* Requested by the project owner, 24 Aug 2026. **No player has been observed
asking for it**, and the two figures that would make it Measured are not collected: how often a
returning player looks for a specific server, and how often a server they starred is absent from
a given sweep. The second one is now cheap to measure — the Favourites status bar computes
"N of M in this list" on every sweep — and is worth capturing before this entry is ranked
against anything above it.
**What Reveille does** A star on every row, and two extra list scopes: **Favourites** and
**History**. A saved server the current sweep did not return is not hidden and not drawn with the
figures it had last time — it says "not in this list" and offers a **Check** button that runs
the same probe against that one address, without a master list. That last part is the point: the
master's list is not the population, and a server missing from it was previously unjoinable in
Reveille at all.
**Status** Shipped. Storage is per-machine `localStorage`; there is no sync and no export.
**What it deliberately does not do** History records that Reveille *launched the game*, not that
the player got in — admission is the server's decision at connect time and no reply reports it
(rule **H12**). And a bookmark stores an address, a query port and a name: no client count, no map,
no round trip, so there is no stale measurement that could be redrawn as current.

### F10 · The figures on the list are older than they look

**Where** Browse.
**Who it stops** Anyone who reads the list, then thinks about it before clicking. A sweep of ~190
registered endpoints takes long enough that a server's client count, map and round trip are already
minutes old by the time a player has compared three of them — and nothing on screen said when they
were taken, so "0 clients" read as *empty now* rather than *empty when this ran*. The only remedy
was **Refresh**, which re-asks every server to update one and loses the selection doing it.
**Evidence** *Assumed, 26 Aug 2026.* Asked for, not measured. What would make it Measured is the
gap between a row's measurement and the join it leads to; that is not collected. The cost of the
old remedy is Measured — a full sweep is a couple of hundred probes (`plan.md:233`).
**What Reveille does** The detail pane says when the row was measured — **From the check at 14:32**
for a row the sweep returned, **Checked at 14:32** for one asked again on its own — and offers
**Check again** (`R`), which runs the same one-request `check_server` probe an absent favourite's
**Check** runs. The time is part of the feature, not decoration: a server that answers with the same
four players it had before would otherwise leave the player unable to tell whether anything
happened.
**What it deliberately does not do** No polling and no auto-refresh of the selected row. A timer
would send requests at third-party servers nobody asked to have watched, and it would move figures
under a player mid-decision. And a check that runs and gets no answer **drops the row** instead of
leaving its figures on screen: the check is evidence about now, and they are not (rule **H12**).
**Status** Shipped.

---

## Stage 3 — Getting in

### F6 · The server runs a map you do not have, and the engine just drops you

**Where** Join.
**Who it stops** **15 of 113** classified live servers need at least one map you would not have
(`plan.md:233`).
**Evidence** *Measured, 19 Aug 2026.* Reproduced end to end on `<[TFC]> Sniper Only OBJ`: 7 of
14 rotation maps present, 9.1 MB across 4 files, 2 awaiting a choice.
**What Reveille does** Prices the download before you commit (`Get 9.1 MB & join`), resolves what
it can, and lists what it cannot as a recorded non-result rather than failing the pass.
**Status** Shipped.
**Narrowed 24 Aug 2026** The **Needs** column was removed from the server list at the owner's
request, so the price is now visible only after a server is selected. The join itself is unchanged
— the button still names the cost, and the gate still runs. What was lost is comparison: a player
scanning the list can no longer see which rows cost a download without clicking each one. If this
turns out to matter, the column is the fix, and `docs/ui.md` §2.1 keeps what it rendered.

### F7 · The server publishes no rotation, so nothing can be promised

**Where** Join.
**Who it stops** **15 of 113** servers publish no `sv_maplist` (`plan.md:235`).
**Evidence** *Measured, 19 Aug 2026.* `=MB= Revival Mie` publishes no rotation and was running
`dm/dm_stanalie`, absent locally.
**What Reveille does** Says `Can't tell` — one checked map is not a rotation check — but still
checks and fetches the map running *now*, so the server is joinable rather than merely unknown.
**Status** Shipped after a review found the running map was being ignored.
**Why this stays honest** The temptation is to upgrade `Can't tell` to `Compatible` once the
current map resolves. That would be a lie; the next map in an unpublished rotation is still
unknown.

---

## Stage 4 — Staying

### F8 · No way to see, update, or remove what you installed

**Where** After the first join, and forever after.
**Who it stops** Unknown — but this is the friction that separates a one-time launcher from a
tool a regular player keeps open.
**Evidence** *Assumed.* The goal — "as easy as installing extensions in VS Code" — is the owner's,
and **no player has been observed asking for it.** Worth an Observed data point before it is
built.
**What Reveille does** Nothing. v1 installs content reactively and re-derives what is present by
walking the disk; there is no installed-state tracking and no way to disable anything.
**Status** **v2**, decided 22 Aug 2026. See `plan.md`, "Scope decided after v1 was drafted".

---

## Believed, but not yet evidence

Parked deliberately. These are the things it would be easy to build on a hunch. Each needs one
Observed or Measured data point before it is ranked against anything above.

- Players give up during the first launch because nothing tells them how long a step will take.
- The four compatibility states are one state too many for a newcomer.
- People want to play Spearhead and Breakthrough more than the map data suggests.
- The ten-minute criterion is the right target. It is the owner's number and has **never been
  measured against a real newcomer** — `plan.md:475` moves the timed test to ship for exactly
  this reason.

---

## Adding an entry

1. Write what stops the player, in a player's words, not the system's.
2. Add the evidence line. If it is *Assumed*, say so — an honest Assumed is more useful than a
   Measured you cannot cite.
3. Say what Reveille does today, including "nothing".
4. Open the issue, and link it back here.
