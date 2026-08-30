<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille interface

This is the authoritative design document for the Tauri shell. It exists because the design
previously lived only at a `claude.ai/code/artifact` URL, an agent working on the shell could not
fetch it, and the interface that resulted had to be thrown away. **Anything needed to rebuild the
interface is here, in the repository, reachable with no network access.** The artifact link in
`AGENTS.md` is a convenience; this file wins where they disagree.

Implementation lives in `crates/reveille-app/ui/`.

---

## 1. What this interface is

A launcher, not a landing page. The player already decided to play; the interface's whole job is to
get them from "the game is on disk" to "in a game with other people" without lying to them on the
way. Everything below follows from two facts about the population it is browsing:

- **Roughly a third of registered endpoints never answer.** The in-game browser lists them anyway,
  which is most of why Allied Assault feels abandoned. Reveille lists only servers that answered,
  and states the difference in the status bar rather than quietly hiding it.
- **Around a quarter of the servers that do answer need content the player lacks, or publish no
  rotation at all.** These are not broken servers. They are usually the interesting ones.

## 2. The two decisions that shape everything

### 2.1 Cost, not verdict

**The server list contains no compatibility badge, no green, and no amber.**

The obvious design is a status column: green *Compatible*, amber *Needs 3 maps*, red *No source*,
grey *Can't tell*. It was rejected, deliberately, because a traffic light teaches one behaviour:
click only green. That is the wrong behaviour here.

"Needs 3 maps" is not a defect. It is the single thing this launcher exists to do, it is one click,
and it takes seconds. Badging it amber next to a green alternative pushes the player away from
roughly a quarter of the live population and specifically away from servers with the richest custom
rotations — reproducing, inside Reveille, the exact "the game is dead" impression Reveille was built
to correct.

So the list states **no compatibility verdict at all**. It carries the facts a server reports —
name, clients, map, ping, build — and nothing that grades it.

**The Needs column was removed on 24 Aug 2026**, by the project owner. It had priced the work in
the list itself (`+ 7 maps`, `7 maps unavailable`, `not published`), colourless except for the one
state a download cannot fix. Removing it costs one thing and is worth writing down: the download
price is now visible **only after a server is selected**, in the detail pane, so a player comparing
rows cannot see which of them costs a download without clicking each one. That is a real
regression against F6, and the reason to restore the column if it is ever missed.

What it does **not** cost is honesty. The four state names were never in the column — they live in
the detail pane, which is where the decision is actually made, and the join gate (§5) is unchanged.
Rule H3 is satisfied there, not here.

**It came back on 27 Aug 2026 and was removed again the same day**, by the project owner. It
returned on exactly the terms the paragraph below sets — a colourless price, never a badge — and it
priced correctly. What it could not pay for was its width. The only place to take 146px from was
**Mode**, either directly or through the breakpoint that drops Mode on a narrow window, and Mode is
what a player narrows the list by before anything else: someone looking for Objective is not served
by knowing what every deathmatch server would cost them. The column answered, on every row, a
question that gets asked about one row.

So the regression recorded above stands, and stands deliberately: **the download price is visible
only after a server is selected.** Restoring the column is not the fix for it. If it is ever missed
badly enough to act on, the thing to reach for is a cheaper answer in the place the question is
actually asked — the detail pane, where the join is decided — not a column charged to every row.

The rejection below still stands with full force: if a *verdict* ever returns to the list, it must
return as a price, never as a badge, a colour or a tick. And a price must arrive with its own
width, not out of Mode's. The table below records what the column looked like both times, so it
does not have to be re-derived.

| State | Cell it used to render | Colour |
|---|---|---|
| `Compatible` | *empty* | — |
| `NeedsMaps { count }` | `+ 7 maps` | none (default ink) |
| `NoSource { count }` | `7 maps unavailable` | `--bad-text`, the only coloured cell |
| `CantTell` | `not published` | `--faint`, italic |
| `CantTell`, current map absent | `+ 1 map` | none (default ink) |

A **Hide unavailable maps** toggle survived the removal for a while, filtering on a state the list
no longer showed. It has since been dropped: a control whose effect is invisible cannot be judged
by the player pressing it, and rows vanishing for a reason nothing on screen states is worse than
the rows being there. The **no "only compatible" filter** prohibition below always covered this
ground anyway, and it covered the Needs column too while that existed: a column prices, it does
not gate.

The toolbar carries two filters, and both gate on a figure the list draws:

- **Not empty** — at least one occupied slot. Called `Has people` until 27 Aug 2026: a slot counts
  while its holder is still connecting or downloading, so a figure of occupied slots is not a count
  of people at play, and the toolbar may not claim otherwise. The state key moved with the label,
  because an identifier that reads as a claim is how the claim gets back onto the screen.
- **Ping under** — a ceiling on the round trip, `Any` by default. Sorting a column is not filtering
  on it: sorting by players surfaces full servers on the far side of the world, and shipping the
  sort without the gate is a documented failure across several modern browsers
  (`docs/ux-standards.md` §7). A server that published no round trip is **not** hidden by a ceiling
  it cannot be measured against.

Whenever either is narrowing the list, the status bar says by how much — `61 of 114 shown`. Without
it the only feedback a filter ever gives is "Nothing matches", and it arrives only once the filter
has hidden everything.

**The four canonical states still exist**, and as of 26 Aug 2026 each is named for what Reveille
measured rather than for how sure it feels: `Compatible`, `Needs N maps`, `No download for N maps`,
`Map list not published`. Three of them are drawn in the detail pane, where the decision is actually
made and there is room to explain them. The list does not repeat them as badges.

`Compatible` is drawn **nowhere**, as of 27 Aug 2026. A ready server's detail pane shows its name,
its address, the map, when it was measured, and a button reading `Join`. A heading saying
`Compatible` over that button restates the control beneath it, and §9's rule — *a ready server says
nothing* — had been written down for a month before the interface obeyed it. Rule H3 is unaffected:
it forbids collapsing four states into a boolean, and a state with nothing to qualify is still not
the same state as one that has.

The two renames replaced verbal hedges — `No source` and `Can't tell` — with the measurement each
one stood for. A hedge expressed as a mood word costs a reader trust in the figure and in the
source; the same fact expressed as a measurement costs almost none (van der Bles et al., PNAS 2020;
`docs/ux-standards.md` §1.1). This tightens the honesty rules rather than relaxing them: the name
now states the observation and leaves the verdict to the player. Enforced in `lib/format.js`
`stateName`.

There is deliberately **no "only compatible" filter**. It would rebuild the traffic light out of
controls instead of colour.

### 2.2 Period flavour in the chrome, never in the data

The WW2 identity lives in: the wordmark, the brass accent, the condensed uppercase field labels,
the near-black ground, first-run and empty states.

It stays out of: tables, numbers, chips, forms, and anything a player reads to make a decision.

The first attempt at this interface pushed the flavour into the data plane — a serif display face
at 75px, film grain over the whole window, olive and rust throughout — and became hard to read. A
launcher is a tool. The game supplies the atmosphere.

## 3. Information architecture

There is **one** primary surface. The three-screen wizard was removed.

```
┌──────────────────────────────────────────────────────────────┐
│ REVEILLE                              D:\Jeux\EA GAMES\MOHDA │  titlebar
├──────────────────────────────────────────────────────────────┤
│ GAME [Spearhead ▾] [All 106│Fav 3│History] ⌕ search  ▓▓▓░ 78/190 │ toolbar
├───────────────────────────────────────┬──────────────────────┤
│ ★ SERVER      CLIENTS  MAP NOW   PING  RUNS │  detail pane    │
│ ★ harzCore      40/64  dm/mohdm6  21ms  1.11 │  server facts   │
│ ☆ <[TFC]>        1/32  obj/bluts  38ms  1.11 │  join check     │
│ ☆ [FORTE]       21/32  dm/mohdm6  14ms  1.11 │  maps           │
├───────────────────────────────────────┼──────────────────────┤
│ 106 of 190 answered · 108 bots · 84 not listed │ [Join]       │  status/actions
└───────────────────────────────────────┴──────────────────────┘
```

- **Setup** (`views/setup.js`) — shown *only* while no install is resolved. Not a welcome page: it
  answers "where is the game", then asks **How do you want to run the game?** with accessible radio
  cards for OpenMoHAA, Reborn, and Original game. Their descriptions are neutral; neither community
  engine receives a recommendation badge. Version, download size, installed state, and active state
  appear only where relevant. OpenMoHAA's stable/preview selector stays inside its card. Continue is
  disabled when the selected engine is unavailable. Reborn installation selects and activates it,
  while switching keeps both managed engines installed. The titlebar chip names the selected game
  and the active engine beside the canonical game folder.
  **When the folder can run more than one game it also asks "Which game do you want to play?"**,
  as a plain radio row above the engine cards — no cards, because there is nothing to say about
  each option but its name. It is asked here rather than left to the toolbar because Continue
  starts a search immediately, and the toolbar's switch is disabled while a search runs: a player
  told to choose afterwards would find the control greyed out. One question with an obvious
  default costs less than an unwanted sweep.
  **Returning here and changing the answer sweeps again.** Setup is re-entered from the titlebar
  chip, and what it changes — the folder, the engine, the game — is exactly what the table on
  screen was the answer to. Continue therefore drops that list and sweeps, by the same rule as the
  toolbar switch: `state.listSession` records what the rows were swept for, and `enterServers`
  compares it. A session that comes back unchanged keeps its list, because re-sweeping it would
  cost a couple of hundred probes to redraw the same table.
- **Game** — a select at the head of the toolbar, labelled **Game**, listing only the games the
  folder can actually run. That is `Installation.playable`, not `products`: an expansion needs the
  base game underneath it, and a folder with `mainta` and no `main` runs nothing (rules **H13**,
  **H14**). **Hidden when there is only one**, which is the common case; a control with a single
  option is noise. It is *not* a filter, and must never be built as one: the three register with
  the master separately and read different directories on disk, so changing it drops the list and
  sweeps again rather than re-labelling what is on screen. Disabled while a sweep is running, and
  while a download is running — a transfer cannot be abandoned half-written. Switching also
  discards the results of anything still in flight, so an install started for one game cannot
  report its outcome into another. The choice is remembered per install folder and named in the
  titlebar chip beside the engine.
- **Servers** (`views/servers.js`) — where the session lives.
- **Scope** (`All` · `Favourites` · `History`) — three exclusive buttons in the toolbar, after
  **Game**. Not tabs: there is still one table, one set of columns and one selection; these
  buttons choose which *view* of the selected game's servers is drawn. **Game** selects the
  population, Scope selects the view — a distinction worth keeping in the copy, because they sit
  next to each other. **This is not a compatibility filter** and must never grow
  into one — see §2.1 on why there is no "only compatible" control. `Favourites` lists what the
  player starred; `History` lists what Reveille launched the game for, most recent first, which is
  a sort key no column header owns, so no arrow is drawn in that scope. **One table means one set of
  column widths**, and the switch may not change them. Every column but **Server** has a fixed
  width, so the name column absorbs any error in the rest of the layout, and it absorbed two:
    - The list pane reserves its scrollbar gutter permanently. Without that, a scrolling `All` and
      a short `Favourites` handed the name column two widths 10px apart.
    - **An absent row's `colspan` counts the columns actually drawn, never `COLUMNS.length`.** The
      narrow-window breakpoints drop Runs, then Mode, then Ping, and a dropped column is gone from
      the table rather than merely invisible — so below 1200px a colspan of `COLUMNS.length - 2`
      overran the row. An overrunning colspan is not clipped: Chromium invents the column that was
      asked for and splits the free width evenly between that phantom and **Server**. Measured at a
      1150px viewport, that halved the name column to 128px and left `Favourites` and `History` —
      the two scopes that have absent rows — visibly narrower than `All`, which has none.
      `columnsShown()` reads the count off the header row, so the media queries stay the one place
      the drop order is written down, and a resize that crosses a breakpoint repaints the rows.
- **Mode** — a column between **Map** and **Ping**, carrying `g_gametypestring` exactly as the
  server spelled it: `Objective-Match`, `Free-For-All`, `Round-Based-Match`. The stock engine sets
  it to one of seven labels (`gamecvars.cpp:560-578`), but it is an ordinary server cvar and a mod
  may publish anything, so the value is **never** mapped onto a fixed set or shortened to
  FFA/OBJ/TDM: an abbreviation Reveille invented would be a claim about a server it cannot check,
  and an unrecognised mode would have nowhere to go. A server publishing none gets the same em dash
  every other unpublished figure gets, and its tooltip says so. Sortable, and blanks sort to the far
  end so a run of dashes cannot bury the modes being scanned. It is a label, not a measurement, so
  it takes the text face rather than the data face and carries no colour, for the same reason
  **Ping** carries none. **The narrow-window drop order is Runs, then Mode, then Ping** — the server
  name is what those breakpoints protect, and two servers you cannot tell apart cost more than any
  of the three.
- **A remembered server the current check did not return** stays in the list as an *absent* row:
  its star, its address, its name in the italic *remembered* style, the words **not in this list**
  across the columns, and a **Check** button. Nothing else — no client count, no map, no round trip, because those were true
  of a past moment (rule **H12**). A bookmark is an address, so it outlives the game it was saved
  under: one that answers for another of the three stays an absent row reading **runs Spearhead**
  and is never promoted to a live server row, because this session's client would be dropped at
  connect. Its **Check** button is replaced by **Switch to Spearhead** — checking again can only
  find the same thing, and the guidance belongs in the row, not in a `title` a keyboard cannot
  reach (§9). When the folder cannot run that game there is no button at all and the row says
  **runs Spearhead, which is not in this game folder**: an action that cannot work is worse than
  none. The button runs `check_server`, the same probe the sweep runs at
  the same deadline; a server that answers becomes an ordinary row and is joinable, and one that
  does not says **did not answer** with the recorded reason in its `title`. Favourites the sweep
  missed are checked once automatically per completed sweep, on first entry into that scope **with
  the absent block open** — collapsed, those probes would write their answers where nobody can
  read them.
- **The absent rows are collapsed behind a disclosure that counts them**, and that is the default
  (rule **H15**). On a folder with more than one game they otherwise dominate the two saved
  scopes for ever: the three games register with the master separately, so a server starred while
  browsing Spearhead is never in an Allied Assault check and its **Check** can only ever find the
  same thing. The disclosure sits in the row where those entries would have been and reads
  **▸ 8 favourites not in this Spearhead check** — the game is named only where the folder has more
  than one, by the same rule that hides the game select. Open, it reads **▾** and the rows follow
  unchanged.
  **This is a fold, not a filter, and it must stay one.** The count is on screen whether the block
  is open or shut, so nothing vanishes for a reason nothing on screen states — the distinction that
  removed **Hide unavailable maps** (§2.1). And it must never fold on a *stored game*: Reveille
  does not know which game a saved address runs, a bookmark is an address (**H12**), and a
  remembered "last seen under Spearhead" would hide a server that has since moved under the very
  game it now runs. The criterion is the one the rows already state — this list does not hold
  them.
  **Check the other N** in the status bar is drawn only while the block is open, because the whole
  of its effect is inside the block. The status bar's own count (**3 of 11 favourites in this
  check**) is taken against everything saved, search box or no search box; the disclosure counts
  what it is actually hiding. The two answer different questions, which is why they can differ
  while a search is typed.
- **Detail pane** (`views/join.js`) — selection previews the join *in place*. No
  browser → join → back navigation; servers stay comparable; the list never disappears.
- **The selected server can be checked again on its own.** Under the address the pane says when the
  row was measured and offers **Check again** (`R`), which runs the same `check_server` probe an
  absent row's **Check** runs — one UDP request, not the couple of hundred a full **Refresh**
  costs. It exists because every figure on a row was measured at one moment and has been ageing
  since: a server that filled up ten minutes ago still reads as empty, and nothing else on screen
  said when. The time is also what makes the control legible when the answer comes back unchanged —
  without it, pressing the button on a server that still has four players looks like nothing
  happened. **Two wordings, because they are two different claims**: **Checked at 14:32** for a row
  this probe re-asked, and **From the check at 14:32** for one the sweep returned, whose rows
  streamed in across the whole run and so were not all measured at its finish time. A check that
  could not *run* says so on its own line, as an alert, and leaves the timestamp alone — the last
  measured figures are still the last measured figures. Hidden while a sweep is running, which is
  already re-asking every row, and refused while a join is running, which owns the pane; **Join** is
  refused in the other direction for the same reason, while a probe for that row is in flight.
- **A check that runs and gets no answer takes the row out of the list.** The figures it replaces
  were true of the last check and have just been shown not to be true now, so they are dropped
  rather than left standing (rule **H12**). In `Favourites` or `History` the row becomes an ordinary
  absent row; in `All` it leaves the table altogether. The selection is kept either way and the
  detail pane says which server it was, in the *remembered* style, what the check found — **Did not
  answer**, **Runs Spearhead**, **Answers at another address** — and offers **Check again**. An
  empty pane after a button press would lose the player's place and explain nothing. **A server that
  moved is not followed**: the selection stays where the player put it and the pane says the server
  now publishes another game address, by the same rule that stops a bookmark being repointed — a
  shared query port is not proof of the same server — and because the new address is often not in
  the scope being viewed.
- Install progress and the launch outcome render inside the detail pane, not as separate screens.

## 4. Honesty rules, as UI rules

These are product contracts, not style preferences. Breaking one is a bug.

The rules themselves live in [`docs/rules.md`](rules.md) with an identifier each; this table
records what *the interface* does to satisfy them, which is the part specific to this document.
Change a rule in the register first, then update the right-hand column here.

| Rule | How the interface satisfies it |
|---|---|
| **H1** · Never call a client count "humans", and never fold bots into it | **Amended 27 Aug 2026:** the column is now `Players` — see `docs/rules.md` H1 for the measurement that allows it. The toolbar filter is **Not empty**, never "Has people"; bots are drawn beside the figure and never added to it. The glossary defines **Players** and **Bots** in reachable text. *Gap: the status bar's qualifier is still a `title` on a span, and the column header carries no tooltip at all — this row once claimed one that `headerCell()` never built.* |
| **H2** · Bots are disjoint from clients | Rendered on their own line as `+8 bots`, never summed into the client figure. The status bar says "counted separately". |
| **H7** · Never imply free slots | Capacity appears only as a denominator (`21/32`). `capacity - clients` is never computed. |
| **H3** · Never emit a boolean "can I join" | Four states, never a tick. `Compatible` is explained as "that is all Reveille can check — the server still decides whether you get in." |
| **H4** · Never report a moh-db download as verified | Candidate rows show `tested` (the catalogue's own flag) and never "verified". Where a server publishes no checksum, the detail pane says an exact-file match cannot be confirmed. |
| **H5** · Never imply the release digest proves publisher authenticity | Visible setup copy promises only that Reveille checks whether the download arrived intact; it never calls the file safe or the publisher verified. The release page's exact file check is optional tooltip detail, not newcomer-facing copy. |
| **H6** · Never state a cause that was not observed | Engine failures are classified in Rust (`OpenMohaaFailureKind`), and since 27 Aug 2026 so are sweep failures (`BrowseFailureKind`: `NoNetwork`, `MasterUnreachable`, `MasterUnreadable`, `Internal`) — never by matching message text in the shell. `NoNetwork` is reserved for local routing, address, or permission failures; a TCP refusal or reset by the remote master is `MasterUnreachable`, never evidence that the player's PC is offline. A per-map catalogue non-result renders as a sentence rather than through `{:?}`. A release that publishes no file check was never downloaded and says so; only a size or digest mismatch may say the download did not arrive intact. An unclassified failure shows its own text rather than borrowing a cause. The original message stays as tooltip detail. |
| **C3** · Never auto-apply an ambiguous match | Choice radios start with **nothing selected**. The total excludes unresolved maps and the pane says how many still need a choice. |
| **H8** · Say where files went | `used_home_fallback` prints the real `%APPDATA%\openmohaa\<game directory>` path, not a euphemism. |
| **H13** · Index the whole search path | An expansion session indexes `main` underneath `mainta` or `maintt`, so a base-game map is never reported as missing on a Spearhead or Breakthrough server. |
| **H14** · Never offer a game the folder has no files for | The **Game** switch lists only the products detected in the folder, and is hidden entirely when there is one. |
| **H9** · A failure is a recorded non-result | Per-map install failures list individually; the pass is never abandoned. Unanswered endpoints are counted and broken down by reason in a dialog. |
| **H10** · Never recommend replacing an installed engine without version evidence | A validated Reveille receipt may say **Up to date** or name another known build. Presence without a valid receipt says **Version unknown**. A current build has no primary engine action; **Reinstall this version** is secondary. |
| **H11** · Never call the measured round trip the in-game ping | The column is **Ping** because that is the word players look for, but every explanation is the honest one: the tooltip says "Time for one status request to this server and back, measured once during this check. Not the in-game ping." The figure carries no colour, no bars and no bands — it is a measurement to sort by, not a verdict, for the same reason the list carries no traffic light. The toolbar's **Ping under** gate names itself after the column, so the control and the figure make the same claim. |
| **H12** · Never present a remembered server's facts as current, and never call a launch a join | A bookmark stores an address, a query port and a name — no figures exist to go stale. An absent favourite says **not in this list** (never "offline": a server missing from the master's list was never asked) and only a check that actually failed says **did not answer**. A live row says when it was measured (**Checked at 14:32**) and is dropped outright when a later check finds the server gone. History says **Launched**, is written only from a launched outcome, and its tooltip says "Whether the server let you in is not something Reveille can see." |
| **H16** · Never offer the installed or an older Reveille release as an update | The titlebar and first-run card show **Update Reveille** only after the updater has compared semantic versions from the latest published manifest. The dialog names both versions and never constructs an offer from release text or a filename. Signature verification is the install gate in S6; it does not sign the manifest's version label. |
| **S2** · Never change engine files while an affected program is running | Installation and Original/Reborn activation are blocked unless the relevant process query confirms stopped. Unknown is blocking, not permission. |
| **S5** · Preserve original executables before installing Reborn | Reborn installation retains first-seen originals. Switching changes the active canonical copies and never describes either managed engine as uninstalled. |
| **S6** · Never replace Reveille from an unsigned release or without the player's choice | A background check can only reveal **Update Reveille**. **Update and restart** is the sole path to installation; **Later** dismisses it. Download progress and **Stop download** stay in the dialog until Tauri has verified the signed payload, after which Windows closes Reveille before replacement. |

## 5. The join gate

**Everything a server publishes about its content is checked — the rotation *and* the map it is
running now.** `classify_server` preflights `sv_maplist` plus `mapname`, deduplicated by `MapKey`,
because the two are not the same set: an admin can load a map directly, and a server can publish a
current map while publishing no rotation at all. Checking only the rotation missed the case that
matters most, and left the running map out of the shopping list so it could not even be fetched.

A server that published no rotation stays `Map list not published` even so. Its one checked map is
real evidence, but calling it `Compatible` would claim a rotation check that never happened. The
detail pane says so in a sentence — "This server published no map list. Reveille checked only the
map it is running now." — rather than under a heading, because the sentence is the whole of it.

**The rotation itself is not drawn** (27 Aug 2026). The detail pane used to head a **Maps** section
and list the published rotation in four groups: on disk, matched in the catalogue, needs your
choice, no download. Three of the four told the player nothing they could act on — a map already on
disk needs no row, a resolvable one is already counted in the button's own label, and *which* maps
have no download changes nothing that can be done about them, so the state name counts them and
stops. Only the fourth group survives, on its own, because it is the one thing Reveille genuinely
refuses to decide: which candidate file a map ambiguously matched. The single map that can block a
join — the one running right now — is named where the block is, in the action bar.

**The gate is about the map running now, not the whole rotation.**

`reveille_core::join::current_map_readiness` classifies the server's `mapname` against local content
as `Playable` / `Missing` / `Unknown`, independently of the rotation verdict. `launch_refusal` in
`crates/reveille-app/src/main.rs` then decides:

- `Compatible` → launch, no consent needed.
- Current map `Missing` → **refuse, even with consent.** The connection would be dropped on arrival.
- Anything else → launch **only** with consent (`accept_incomplete`).

A server with one unobtainable map later in its rotation is perfectly playable until the rotation
reaches it. Refusing that join would invent a problem the engine does not have; the honest thing is
to state the consequence — "you will be dropped when the rotation reaches this map" — and let the
player decide.

**The gate runs after the install, never before it.** `install_and_launch` downloads, rescans the
map index, re-classifies, and only then calls `launch_refusal`, so a current map that was missing
but fetchable is present by the time the gate is evaluated.

**The interface must not pre-empt that.** The action bar hard-blocks only when the current map is
missing *and* the catalogue has no source for it (`currentMapFetchable` in `views/join.js`, which
lines the server's `mapname` up with its rotation entry using the engine normalisation from
`format.js`). When the running map is in the shopping list, downloading is precisely the fix, and
disabling the button would strand the player on the one screen that could have solved it — the
action bar says so instead: *"X is running now and is not on disk. Fetching is what makes this join
work."* When candidates exist but none is chosen, it asks for the choice rather than refusing.

**Consent is the click on the primary button, and the label is what makes it informed.** There is
one control, and it names what this join is missing:

| Situation | Label | `accept_incomplete` |
|---|---|---|
| `Compatible` | `Join` | `false` |
| Anything to fetch | `Get 9.1 MB & join` | `true` |
| `Map list not published` | `Join without a map list` | `true` |
| Nothing fetchable left | `Join anyway` | `true` |

An earlier version put a separate confirmation toggle in front of the button. It was two clicks for
one answer: the state name sits directly above the button and the label already states the cost, so
the toggle asked the player to agree to something they had just read and were about to act on. What
must never happen is the *silent* inference the first implementation did — deriving the flag from
the state with no label change, so a player launched an unchecked join without being told. The label
is the telling.

## 6. Visual system

Tokens live in `ui/styles/tokens.css`. Ratios below are measured against `--bg` / `--panel` /
`--rise`.

| Token | Value | Role |
|---|---|---|
| `--void` | `#090B0E` | titlebar, status bar, inset fields |
| `--bg` | `#13171C` | window ground |
| `--panel` | `#1A1F26` | toolbar, detail pane, cards |
| `--rise` | `#212831` | raised controls |
| `--line` / `--line-strong` | `#262E38` / `#323C48` | rules, borders |
| `--ink` | `#E7E5E0` | primary text — 14.29 / 13.16 / 11.81 |
| `--dim` | `#98A1AC` | secondary text — 6.88 / 6.33 / 5.68 |
| `--faint` | `#8A929E` | tertiary text — 5.73 / 5.27 / 4.73 |
| `--faint-deco` | `#626B76` | **decoration only** — 3.33, never text |
| `--brass` | `#D9A648` | accent, primary action, wordmark — 8.14 |
| `--ok` `--warn` `--bad` | `#5CA372` `#D98040` `#C4594C` | dots, borders, fills |
| `--ok-text` `--warn-text` `--bad-text` | `#7FC294` `#E8A06B` `#E08375` | the **only** state colours allowed on type |

Every text token clears WCAG AA (4.5:1) on every surface it is permitted on. `--bad` measures 4.18
and is therefore forbidden on text — that is what `--bad-text` is for. Re-run the check when
changing any value.

**Type.** No bundled font files and no CDN. A desktop app must render before any network exists,
and bundling adds binaries and licence obligations to a GPL repository for no gain on the only
platform v1 supports.

- Display / labels: `Bahnschrift` — the condensed face Windows 11 ships — falling back to
  `Segoe UI Variable Display`, `Segoe UI`, `system-ui`.
- Body: `Segoe UI Variable Text`, `Segoe UI`, `system-ui`.
- Data: `Cascadia Mono`, `Consolas`, `ui-monospace`. Every number, path, address, map name and
  filename is monospaced with `tabular-nums` so columns align and figures compare.

## 7. Accessibility and interaction

- The server list is a real `<table>` with `<caption>`, `scope="col"` and `aria-sort`, carrying
  `role="grid"` with the row and cell roles written out. The role is load-bearing twice over:
  `aria-selected` is not supported on `row` inside the plain `table` role, so the app's primary
  interaction was invisible to assistive technology, and a grid is a **composite** widget, which is
  what licenses the single tab stop below.
- **One tab stop for the whole table.** Every row carried `tabindex="0"` until 27 Aug 2026, which
  put the Join button roughly 260 Tab presses from the search box and made the arrow keys the table
  was designed around redundant. Roving tabindex: the selected row is `0` — the first row when
  nothing is selected — and every other row and every control inside a row is `-1`. Guarded by
  `the_server_table_is_one_tab_stop`.
- **Selection follows focus**, which is the grid convention for a single-select list and what makes
  the arrow keys worth having. It is not the network storm it used to be: focus now moves only on a
  deliberate arrow press, and the catalogue lookup behind a selection waits ~220 ms for the
  selection to hold still (`PREVIEW_SETTLE_MS` in `app.js`). Holding `↓` through twenty rows sends
  one request, not twenty.
- **Keyboard**: `↑`/`↓`/`Home`/`End` move between rows — *every* row, so the absent block's
  disclosure and its **Check** buttons are reachable at all. `→` steps into the row's controls and
  along them; `←` steps back and, from the first control, returns to the row; `Escape` does the same
  from anywhere inside a row. `Enter` and `Space` activate. `/` and `Ctrl+F` focus search, `Escape`
  clears it, `F6` cycles toolbar → list → detail pane, `F5` or `Ctrl+R` refreshes the whole list,
  `R` checks the selected server on its own, `F` stars or unstars it. The modifier is the
  difference between one probe and a couple of hundred — which is why `R` must never fire from
  inside a control that has its own use for the letter.
- **Right-clicking a row opens Reveille's own menu**, not WebView2's Back/Reload/Inspect — the
  loudest "web page in a costume" tell a Tauri app can produce, sitting exactly where the native
  convention for this kind of application puts bookmarking. `Shift+F10` and the Menu key open it
  from the keyboard. Everything in it is reachable another way; a context menu that *owns* an
  action is a trap for anyone who does not think to right-click. The browser menu is left alone
  over anything selectable, where Copy is genuinely what a right-click is for.
- **A control that disables itself on click uses `aria-disabled`, not `disabled`.** Focus cannot be
  restored to a disabled element after a repaint, so **Check again** — which goes busy the instant
  it is pressed — would take the caret with it every time. It keeps its place in the tab order and
  its handler refuses the second press (`canRecheck` in `lib/store.js`, read by both the control and
  the handler so the two cannot disagree). `.btn[aria-disabled="true"]` is styled exactly like
  `.btn:disabled`.
- **A single-server check is announced.** The `role="status"` region reports the check starting, a
  command that could not run, and a server dropped from the list — otherwise the one interaction
  with no progress meter, and the one that can remove the row the player is looking at, would be
  silent. Only the *selected* server is announced; a favourites batch would bury the sweep summary.
- Bare-letter shortcuts (`/`, `F`, `R`) fire only when the event target is not a form control, not
  contenteditable, and not inside an open dialog, and only with no Ctrl/Alt/Meta held. `R` on the
  game `<select>` is that control's own type-ahead, not a re-check.
- A row contains buttons — the star, and **Check** on an absent row — so the tbody key handler
  **returns early when the event target is a button**. Without that it swallows `Enter` and
  `Space` and the controls work with a mouse and are dead to a keyboard. The cost is that `↑` and
  `↓` do not move rows while focus is inside one of those buttons, which is the right trade: a
  control that cannot be activated is worse than one that cannot be escaped by arrow. Roving
  tabindex preserved it, and `←`/`Escape` now give the way out that trade used to lack.
- **The sort buttons in the header stay in the ordinary tab order** rather than joining the roving
  tabindex. There are five of them, not two hundred, and folding them in would make the one control
  that reorders the list reachable only by first entering the rows it reorders.
- `:focus-visible` shows a brass ring on every interactive element. Outlines are never removed.
- A `role="status" aria-live="polite"` region announces sweep progress and state changes.
  `role="alert"` is reserved for genuine errors. **Sweep progress is coarsened to quarters** before
  it is announced — start, three milestones, then the summary — and the milestone text deliberately
  carries no running count, which would make the string differ on every probe and defeat the point.
  A sweep emits one event per probed endpoint, so restating "N of M done" on each sent a screen
  reader roughly two hundred announcements per sweep, which `docs/ux-standards.md` §5.7 calls a
  denial of service against the one output a blind player has. Guarded by
  `sweep_progress_is_announced_at_milestones_rather_than_per_probe`.
- All motion respects `prefers-reduced-motion`.
- Progress is determinate wherever a total is known (`78/190`, byte counts) and indeterminate only
  during the master handshake, where nothing is known yet.
- Every long operation is cancellable: the sweep has a **Stop**, selecting another server
  abandons the in-flight catalogue lookup, and the OpenMoHAA download has **Stop download**. Engine
  cancellation is checked between response chunks and never interrupts the atomic apply phase.
- Whether a release-owned program is running is probed **after** the archive has downloaded, not
  before. The
  transfer is long enough for a player to start the game inside it, and a stale reading turns an
  honest request to close that program into a locked-file error part-way through the apply. The
  probe covers every executable a release archive replaces, not only `openmohaa.exe` — a running
  dedicated server holds the same files. Copy distinguishes the game, server and launcher from the
  process name; it does not call a dedicated server the game.

### Re-rendering must not steal the caret

`lib/dom.js` exports `preserveFocus(region, repaint)`. Replacing a subtree detaches whatever had
focus, and the first version of this interface rebuilt the toolbar on every state change — which
made the search field accept exactly one character before dropping the caret. Interactive elements
that survive a repaint carry `data-focus-key`; the toolbar is built once and updated in place. **Do
not reintroduce a blanket rebuild of any region containing a text input.**

## 8. Implementation notes

- **No framework, no bundler, no npm runtime dependency.** `tauri.conf.json` serves `ui/` verbatim;
  `cargo run -p reveille-app` is the whole dev loop. Native ES modules.
- **DOM is constructed, never interpolated.** `el()` in `lib/dom.js` builds every node. Server
  hostnames and map names are arbitrary third-party bytes and must never reach `innerHTML`.
- **`lib/api.js` is the only module that knows the Rust contract** — commands, events, payload
  shapes. A signature change has one place to land.
- **CSP is enforced** (`tauri.conf.json`): `script-src 'self'` with no `unsafe-inline` and no
  `unsafe-eval`. `style-src` permits `unsafe-inline` solely because progress meters set a computed
  width; no untrusted value is ever interpolated into a style.
- **Streamed rows are pre-deduplication.** `browse_streaming` emits outcomes as they arrive, but
  duplicate game endpoints are demoted only once the sweep completes, so retention stays
  deterministic. The payload returned at the end replaces the streamed list with the authoritative
  one. A consumer that shows streamed rows must reconcile against it.
- **The table header is written in place, never rebuilt.** Its sort arrow and `aria-sort` come from
  `state.sort` on every render. Building the cells once and reading `state.sort` at construction
  froze both on whichever column happened to be sorted when the view was created, so clicking a
  header re-sorted the rows while the arrow stayed put. The same rule as the toolbar, for the same
  reason.
- **`version` vs `game_version`**: the list uses `game_version` (`1.11`, `1.12+0.83.0`) because it
  is short and comparable. `version` is a sentence — "Medal of Honor Allied Assault 1.11 win-x86
  Mar 5 2002" — which truncates to "Medal of Honor Allied" in every row and distinguishes nothing.
  The full string is the row's tooltip. The detail pane does not repeat either: compatibility is
  what the player is deciding, and the join verdict states it outright, so a version number there is
  a fact with no next click attached.

## 9. Copy

Plain language, no jargon, no exclamation marks, and never a claim the protocol cannot support.
Say what was checked, what was not, and what happens next.

**Honesty is a constraint on claims, not a licence to explain.** The first version of this interface
satisfied every rule below and was still wrong: it argued its reasoning at the player. Every list
had a paragraph justifying it, every server carried the same standing note about bans and capacity,
every ambiguous match re-explained the no-auto-apply policy. None of that changes what a player does
next, and the volume buried the two or three lines that do.

The rule: **an explanation earns a paragraph only if it changes the next click.** Otherwise it is a
`title` attribute on the thing it explains, or it is not in the interface at all.

- A caveat that is true of every server (bans, capacity, ping) belongs in the docs, not on every
  row. State it once at the moment it applies — after launch, not before every join.
- A group heading plus its data usually says enough. "Needs your choice" over two candidate rows
  with sizes and download counts needs no prose; the policy behind it is the heading's tooltip.
- A ready server says nothing. Silence is the correct rendering of "nothing to do".
- Prefer the shorter true sentence. "Publishes no map checksum, so only names are matched, not
  files" beats the same fact in three clauses.
- Translate implementation terms into the decision they support. Newcomer-facing copy says
  "checks that the download arrived intact", not `SHA-256`, digest, GitHub API, asset, or archive.
  Exact filenames and published digests may remain optional tooltip detail for diagnosis.

- Say "Launched", never "Joined" or "Played". Reveille started the game and saw it start; the
  server decides admission at connect time and never tells Reveille the answer.
- Say "not in this list" for a saved server the sweep did not return, and "did not answer" only
  after a check actually ran and failed. The two are different facts: most absences mean the
  server was never asked.
- Say "clients", never "players".
- Say "did not answer", not "offline" — we know the former, not the latter.
- Say "not published" when a server published nothing. Silence is shown as silence and never
  upgraded to a tick.
- State consequences plainly: "you will be dropped when the rotation reaches this map" beats a
  warning icon.
- Never imply Reveille controls admission. Bans, capacity and ping limits are decided by the server
  at connect time, and the interface says so.
