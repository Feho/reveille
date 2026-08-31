# Design review, 26 Aug 2026

An adversarial UI/UX review of v1, cross-checked against `docs/ux-standards.md`. Findings below
were verified against the source; the file and line references were current at the time of the
review and are the place to start, not a guarantee.

**Context that changed the frame.** v1's recorded bar is "working end to end on the owner's Windows
machine, not packaged, not signed" (`docs/plan.md:443`). The goal is now **fast adoption by the
MOHAA community**, and `docs/*.md` is revisable rather than fixed. Both reviews were run against
the old bar; where the new goal changes the ranking, it is said so below.

---

## Verdict

The honesty model is not decoration: it is enforced in types, in copy, and in layout, and no place
was found where it is broken to make something look nicer. Against its own recorded bar, v1 is
done.

It is not yet ready for the audience in `docs/vision.md`, for one reason above all others:

> **The sentences that make Reveille's rigour legible are almost all `title` attributes.**
> `Can't tell`, `No source`, `Ping`, `not in this check`, `+8 bots` reach the screen as bare
> two-word nouns; the honest explanation that turns each into a decision lives in a tooltip a
> beginner will not hover, a keyboard cannot reach, and a touch device cannot fire.

The project spent its budget being right and then hid the part that communicates it. That is a
copy-and-markup problem, not a structural one.

The research pass then sharpened the fix. Adding explanatory prose beneath a mood word is the
second-best answer; the best one is to **replace the mood word with the measurement** — see F3.

**Nothing found here requires rebuilding anything.**

---

## Blockers

### F1 · Packaging is the real adoption blocker

`docs/plan.md:443` scopes v1 to an unpackaged local build, and `tauri.conf.json` has
`"bundle": { "active": false }`. Under the old bar that was correct. Under "quickly adopted by the
community" it is not: a build a player cannot download and run has no users, and every copy fix
below it is unreachable.

- **Ship NSIS or MSI, not MSIX.** Tauri's Windows bundle targets are `msi` and `nsis`; MSIX needs a
  second toolchain, buys nothing for the Store (which has accepted unpackaged Win32 apps since
  2021), and its write virtualisation fights the one part of Reveille that is already the most
  environment-sensitive.
- **Signing.** SmartScreen reacts to the binary, however it arrived, so winget not requiring a
  signature is irrelevant. Reveille is GPL-2.0-only and so qualifies for Certum's open-source
  certificate; SignPath has a free OSS tier; Azure Trusted Signing is cheap if the eligibility
  rules fit. Sign every binary, not only the installer (`docs/ux-standards.md` 7.5).
- **The GitHub release page is a primary interface surface.** winget and the Store serve power
  users; a returning player finds Reveille through a Discord or forum link. That page is where the
  trust decision is actually made, and it is currently outside every review.
- **WebView2** is a first-run dependency on Windows 10 with no copy for its absence
  (`docs/ux-standards.md` 7.6).

*Open: what the community's trust threshold actually is — whether an unsigned build from GitHub
reads as normal or as a virus — is unmeasured. It decides whether the certificate is a v1 line item
or a nice-to-have.*

### F2 · Nothing tells a player what Reveille needs on disk

Grepping the views for `gog|steam|disc|game files|data` returns nothing. A player on a fresh PC
gets "Show Reveille your game" (`views/setup.js:48`), points at a folder, and gets "No Medal of
Honor installation there." (`setup.js:249`). Loop. No error names the cause, because from the app's
point of view nothing failed.

Setup offers to install OpenMoHAA — an *engine*. It cannot install the game **data**, which
`install::identify` requires (`src/main.rs:428`). That boundary is never stated.

This is `docs/friction.md` F1, and the hardest possible wall: no error, no next action, at second
zero.

**Prior art solved this.** Doomseeker auto-downloads PWADs but requires the player to already own
the IWADs, and states it as a precondition up front rather than as a mid-flow error
(`docs/ux-standards.md` 7, prior art). That is exactly this boundary.

**Fix.** State the prerequisite before any effort is spent, and name the folder to look for.
*Done — see the implementation note at the end.*

### F3 · The four states are mood words, and the explanation is unreachable

Two findings that turned out to be one.

`stateExplanation()` reaches the screen only as a `title` on the `<h3>` (`views/join.js:301`).
So does the ping sentence (`lib/format.js:66`), the *No source* consequence (`join.js:426`), the
folded-rows explanation (`views/servers.js:622`), and the clients caveat — which is a `title` on a
**status-bar span** (`servers.js:831`).

`docs/ui.md` section 3 already states the rule — guidance "belongs in the row, not in a `title` a
keyboard cannot reach" — and then does not apply it to the states, the ping, or the consequence of
*No source*. It is also a **WCAG 2.2 AA failure**, not a preference: SC 1.4.13 requires hover
content to be dismissible, hoverable and persistent, and a `title` fails all three by construction
(`docs/ux-standards.md` 3.1).

`docs/ui.md:252` additionally claims H1 is satisfied because "The column is **Clients**. Its tooltip
says..." — **`headerCell()` builds `th` from `scope` and `className` only** (`servers.js:414-454`).
There is no such tooltip. The compliance table describes an interface that does not exist.

**But the fix is not simply to promote the sentence.** `Can't tell` and `No source` are mood words,
and the strongest evidence in `docs/ux-standards.md` (1.1) says a verbal hedge costs trust while a
measurement does not:

| Was | Now |
|---|---|
| `Can't tell` | `Map list not published` |
| `No source` | `No download for 1 map` |
| `Needs 3 maps` | unchanged — already a count |
| `Compatible` | unchanged, and still silent |

This is cheaper than adding layout, and it strengthens the honesty rules rather than diluting them.
*Done — see the implementation note.*

### F4 · Every row is a tab stop, and tabbing selects it

`row()` sets `tabIndex: 0` on every `<tr>` (`servers.js:515`) with `onfocus: choose`
(`servers.js:518`), and each row also contains a real `<button>` star (`servers.js:481-500`). No
roving-tabindex logic exists.

With around 130 answering servers, reaching the Join button from the search box costs roughly 260
tab stops. Worse, `onfocus` fires `select()` on each one, and for every non-`compatible` row
`select()` invokes `preview_join` (`app.js:222-254`) — an uncancellable moh-db resolution
(`main.rs:1243-1260`; the client-side `previewToken` discards the *answer*, not the request).
Arrowing down twenty rows fires twenty catalogue lookups at a third-party service.

`docs/ui.md` section 7 designed arrow-key navigation precisely because the table is one composite
widget — but a composite widget must be **one** tab stop. As built, the arrow keys are redundant and
Tab is a trap in all but name. It also silently contradicts section 7's cancellation promise.

**Fix.** `role="grid"` plus roving tabindex: the selected row gets `tabindex="0"`, every other row
`tabindex="-1"`. Star buttons get `tabindex="-1"`, reached by arrow or by the existing `F`
shortcut. Debounce `select()` by about 200 ms so arrow-scrolling does not fire a lookup per row,
and give `preview_join` a cancellation token. Set `aria-rowcount` if rows are ever virtualised
(`docs/ux-standards.md` 6.5).

This also fixes F11 — the same change, once.

*Done — see the implementation note.*

### F5 · "Check" means four different things across thirteen strings

"Check this folder" (`setup.js:235`), "Join check" (`join.js:299`), "Check again" (`join.js:170`),
"Check" (`servers.js:725`), "Check the other 8" (`servers.js:891`), "not in this check"
(`servers.js:763`), "8 favorites not in this Spearhead check" (`servers.js:631`), "From the check
at 14:32" (`join.js:205`), "3 of 11 favorites in this check" (`servers.js:877`), "checking
archive" (`join.js:527`), "The check could not run" (`join.js:169`), "Checking servers"
(`servers.js:786`), "Back to the check" (`join.js:711`).

Four meanings: identify a folder, assess compatibility, probe one server, run the whole sweep —
plus verify a download. And the toolbar button for the sweep says neither, it says "Find servers"
and "Refresh" (`servers.js:204`, `:296`), so the player has no word at all for the thing the status
bar calls "this check".

This is the highest-frequency vocabulary in the app and the vocabulary a non-native reader has
least slack for. "8 favorites not in this Spearhead check" is unparseable on first read. It
violates the one named rule in federal plain-language guidance (`docs/ux-standards.md` 2.1).

**Fix.** Reserve *check* for one meaning — asking one server — and rename the rest: the sweep
becomes **server list**; `Join check` becomes **Before you join**; `Check this folder` becomes
**Use this folder**; `Back to the check` becomes **Back to server details**; `checking archive`
becomes **checking the file**. No honesty is lost: "not in this list" is exactly as precise as "not
in this check".

*Done — see the implementation note.*

### F6 · Raw errors reach the player, under a contradicting empty state

`main.rs:1391` formats a per-map failure with Rust `Debug`:
`format!("the catalogue lookup did not complete: {:?}", result.reason)`, rendered at
`join.js:600-612`.

Every network failure path calls `.map_err(|error| error.to_string())` and lands in `errorText()`
(`lib/api.js:88`), which can produce "GameSpy encryption key is empty", "master reply body has 42
bytes; expected a multiple of 6", "moh-db returned HTTP 503". These render as the *entire* status
bar (`servers.js:819`) and as the live-region text (`servers.js:969`).

Compounding it: while `browse.error` is showing, the table renders `emptyRow()`'s last branch —
"No servers yet / Nothing has been checked yet." (`servers.js:800-808`). Something *was* checked and
it failed. The centre of the window contradicts the corner, and neither offers a next action.

These are the two moments where a non-technical player decides whether the tool is broken or their
PC is: no internet, a firewall blocking UDP, a captive portal.

**Fix.**
1. Classify browse failures in Rust the way `OpenMohaaFailureKind` already classifies engine
   failures — at minimum `NoNetwork`, `MasterUnreachable`, `MasterUnreadable`, `Other` — each with
   a player sentence carrying cause and remedy. Keep the original string as detail.
2. Replace the `{:?}` at `main.rs:1391` with a `Display` rendering.
3. **Do not blank the table on a failed sweep.** Keep the previous rows, marked stale
   (`docs/ux-standards.md` 4.5) — this goes further than adding an error branch to `emptyRow()`,
   and is the honest product beating the conventional one.
4. Grep the error catalogue for "please", "sorry", "valid" and "invalid"
   (`docs/ux-standards.md` 4.2).

*Done — see the implementation note.*

### F7 · "Has people" breaks H1, in the same window as the tooltip that forbids it

`toggle("Has people", ...)` at `servers.js:174`, filtering on `clients_reported >= 1`
(`lib/store.js:293`).

H1 is "never call a client count 'players' or 'humans'". `lib/format.js:8-10` restates it in its own
header comment. `servers.js:831` renders "Occupied slots reported by every server. **Not verified as
people.**" The toolbar toggle, four inches away, claims exactly that.

This is the one honesty rule the interface states out loud and then contradicts on screen.
`docs/ui.md` section 2.1 blessed the label without checking it against the register, and
`docs/rules.md` "Known gaps" already warns that H1-adjacent wording has no mechanical guard.

**Fix.** `Not empty`. Shorter, honest, and clearer to a non-native reader than "Has clients", which
re-uses a word the player is still learning.

*Done — see the implementation note.*

### F8 · The download price is invisible until you click, and clicking costs seconds

To answer "which of these can I just join, and which need a download?", a player must click each
row, wait for a moh-db resolution (`join.js:325-343`), read the price, and click the next.

`docs/ui.md` section 2.1 records this honestly as "a real regression against F6, and the reason to
restore the column if it is ever missed". F6 is **Measured** (15 of 113 servers), and it is the
friction the vision statement names.

Section 2.1's rejection of a badge is correct and should stand. But it also draws the distinction
that saves this: a **price** is not a verdict. `+ 7 maps` in default ink grades nothing; it tells a
scanning player what the click will cost. `docs/ux-standards.md` 1.5 backs this directly — prefer a
countable quantity over a graded judgement, especially for low-numeracy readers.

**Fix.** Restore the **Needs** column as `docs/ui.md` section 2.1's table records it, including the
colourless treatment of `NeedsMaps` and `--bad-text` reserved for `NoSource`. The table currently
carries seven columns (`servers.js:49-66`), so Needs likely displaces `Runs`.

*Done — but Runs was **kept**, not displaced. It is the last place the server's build string
appears anywhere, the Engine row that used to carry it in the detail pane having been dropped on
24 Aug 2026; displacing it would have quietly reversed that decision. The first breakpoint moved
out from 1200px to 1360px instead. See the implementation note.*

---

## Should fix soon

### F9 · No glossary anywhere
Reveille runs a closed lexicon of terms of art — *listed*, *replied*, *not in this check*,
*launched*, *clients*, *bots*, *rotation*. None is defined anywhere a player can reach. WCAG
**SC 3.2.6 Consistent Help is Level A**, and a panel reachable from the same place on every view
satisfies it while attacking F3 and F5 at the root rather than patching thirty strings.
*Done — see the implementation note.*

### F10 · An Assumed feature is ranked above a Measured one, by layout
`header()` puts the freshness row and **Check again** inside `.detail__head`, above `facts()` and
`verdictSection()` (`join.js:88-133`). `docs/friction.md` F10 is *Assumed*; F4, the join verdict, is
*Measured* — "Who it stops: Everyone, on every server, every time." The ledger's own rule is that
Assumed entries may not be ranked above Measured ones, and visual position is a ranking.

Partly in tension with `docs/ux-standards.md` 5.6: for an explicit-refresh product, data age is
load-bearing. **Demote the button, keep the timestamp** — `R` already covers the power path.

### F11 · The table announces a selection nothing can hear
Rows carry `aria-selected` (`servers.js:516`, `:319`) inside a plain `<table>` with no
`role="grid"`. `aria-selected` is not supported on `row` in the `table` role, so the app's primary
interaction is invisible to assistive technology. Same fix as F4. *Done.*

### F12 · The product's core claim is whispered
`docs/ui.md` section 1 says the registered-versus-answered difference is stated in the status bar
"rather than quietly hiding it". That status bar is `--text-xs` (11 px), `--faint`, monospace, at
the bottom edge (`styles/views.css:419-433`).

"106 of 190 answered · 108 bots, counted separately · 84 registered but not listed" is the most
product-defining sentence Reveille writes, rendered at the smallest size in the second-lowest text
token in the least-scanned corner. Contrast passes; legibility does not follow from contrast
(`docs/ux-standards.md` 3.4 — readers take in about 20% of words).

It is also the antidote to the single best-documented failure in this whole product category:
Steam's browser silently omitting servers that are online, which players read as "the game is
dead" (`docs/ux-standards.md` 7, prior art).

**Fix.** Keep the position, raise the register: `--text-sm`, `--dim`, and let the answered figure
use `--ink`. Costs about 8 vertical pixels.

### F13 · Search behaves differently in the two halves of the same list
`matchesFilters()` matches `hostname` only (`store.js:291-296`); `partitionScope()`'s absent branch
matches `hostname` **or** `address` (`store.js:343-345`). Pasting an IP in **All** gives "Nothing
matches" while the server is on screen; the same paste in **Favorites** finds it.

**Fix.** Add `|| row.address.includes(query)` to `matchesFilters`. *Done.*

### F14 · No search match count
"Nothing matches" exists (`servers.js:781`, `:791`) but the populated case never says "38 of 214".
That count is the cheapest defence against "why is it empty?" (`docs/ux-standards.md` 2, table 14).
*Done — the status bar says `61 of 114 shown` whenever a filter is narrowing.*

### F15 · Ping is a sort, never a filter
`docs/ux-standards.md` 7 names shipping-sort-without-filter as a documented failure across several
modern browsers: sorting by clients surfaces full 250 ms servers, sorting by ping surfaces empty
nearby ones. Doomseeker has had a ping threshold since the 2000s. Reveille has the column
(`servers.js:59`) but no gate. *Done — a **Ping under** ceiling, `Any` by default. A server that
published no round trip is not hidden by a ceiling it cannot be measured against.*

### F16 · No way to reach a server by typing its address · *Assumed*
`check_server` already takes an arbitrary address and query port (`api.js:65`), but the interface
only ever calls it for entries already in `favorites()` or `history()`. A player handed an IP in
Discord — *Assumed* to be how most MOHAA servers are shared — has no entry point, and no way to
create the bookmark that would give them one. F9's own premise argues for this and the backend
exists.

*If this matches what the community actually does, it deserves an F-entry and probably outranks
F10.*

### F17 · Setup copy is written for someone who already knows the answer
`views/setup.js:16-21` — "Modern rebuilt game program." (rebuilt from what?); "Existing game
program without community-engine updates." (a compound a non-native reader cannot decompose).
`:55` the disabled primary button reads "Choose an available engine", but the word *engine* appears
nowhere else on that screen, where the cards all say *game program*. `:129` "`v1.12` is installed.
Reveille will not call it current." is H10 speaking in its own voice. `:128` is a visible `.note`
that changes no click, which section 9 says belongs in a `title` or nowhere.

F2 in the friction ledger is *Measured*: most servers still accept a retail client, so neutrality is
defensible. But neutral is not the same as unhelpful, and nothing in `docs/rules.md` forbids stating
a *consequence* rather than a recommendation. Rewrite each as what the player gets — "Rebuilt to run
on modern Windows. Actively maintained." / "The original game with community fixes." / "The game
exactly as you have it. May need manual fixes on Windows 10 and 11."

### F18 · "contacting master" is shown while the app reads the disk
`browse_servers` calls `installed_maps(&session)` — a full pk3 walk — *before* the sweep starts
(`main.rs:876`). During that time `inspected === 0`, so the meter reads "contacting master"
(`servers.js:308`). In a project that classifies failure causes in Rust so the shell can never guess
one (H6), guessing a *progress* cause is off-key. Emit an indexing phase, or say "reading your
maps" (`docs/ux-standards.md` 5.3).

### F19 · An expansion-only folder walks into a dead sweep
`Installation.playable` can legitimately be empty (`reveille-core/src/install.rs:92-95`). Then
`gameChoice()` returns false because `games.length < 2` (`setup.js:76-77`), `foundBlock` lists
"Spearhead", Continue is enabled, and `defaultGame()` falls back to the literal `"allied_assault"`
(`store.js:219`). The first sweep dies with "Allied Assault cannot be run from this game folder"
(`main.rs:1151-1154`) — under F6's contradicting "Nothing has been checked yet."

H14 is honoured by the toolbar switch and bypassed by the default. Refuse Continue when
`playableGames(install).length === 0`, and say so.

### F20 · Four labels a beginner will misread
- `join.js:794` "Join without a rotation check" — *rotation* is jargon; `docs/ui.md` section 5
  deliberately heads the section *Maps* for that reason, then the button reverts.
- `join.js:427` "No source" as both state name and group heading. Superseded by F3.
- `join.js:733` "Cannot join yet", disabled — "yet" promises a change that cannot come.
- `join.js:576` "Bans, a full server and ping limits are the server's call from here." Idiomatic;
  "the server's call" will not survive translation (`docs/ux-standards.md` 2.7).

Note: replacements must not introduce negative contractions (`docs/ux-standards.md` 2.3). The
codebase has three — `join.js:7`, `format.js:146`, `servers.js:623`.

*Done. The two remaining are in source comments quoting the old state names; the one player-facing
contraction, in the disclosure's tooltip, is gone.*

### F21 · The only route back to Setup is an unlabelled chip
`#install-chip` (`index.html:18-26`) has `title="Game folder, selected game, and active engine"` —
a description of what it shows, not that clicking changes them. It renders as 11 px `--dim`
monospace with `direction: rtl` (`views.css:34-60`). A player who picked the wrong engine has no
visible way back.

### F22 · Windows conventions and webview tells
`F5`, `Ctrl+R`, `/`, `F`, `R` and `Escape` are bound (`app.js:530-560`) — good. Missing:
- **`Ctrl+F`** for search and **`F6`** to cycle table, detail pane and toolbar.
- **The WebView2 context menu is not suppressed** (`grep contextmenu` returns nothing), so
  right-clicking a row offers Back, Reload and Inspect. This is the loudest "web page in a costume"
  tell, and Doomseeker's right-click-to-bookmark is the native convention being missed
  (`docs/ux-standards.md` 7.3).

*Done — both, plus a Reveille context menu carrying only actions reachable another way.*

### F23 · Verify the live region is not streaming
The sweep text is "Checking servers, N of M done" (`servers.js:967`). If that re-renders per probe,
a screen reader receives around 200 announcements, which `docs/ux-standards.md` 5.7 calls a denial
of service. Announce start, coarse milestones, and the summary.

*Verified: it did re-render per probe. Now announced at quarters, with no running count inside the
milestone text — a count would make the string differ on every probe and defeat the milestone.*

---

## Considered and rejected

Things that look like findings and are not. Several were tempting to "tidy up" until the docs
explained why not.

- **No compatibility badge, no green/amber, no "only compatible" filter.** `docs/ui.md` section
  2.1's argument holds: a traffic light would push players away from a quarter of the live
  population and reproduce the "MOHAA is dead" impression the product exists to correct. F8 asks
  for the *price*, not the verdict. Independently confirmed by `docs/ux-standards.md` 1.5 and 1.9,
  and by prior art where such warnings get dismissed.
- **~~`Clients`, not `Players`.~~ Reversed 27 Aug 2026.** This read "H1. Less friendly, and the
  only honest word." H1's stated grounds — no way to tell a bot from a person in the figure — are
  contradicted by the project's own live measurement: bots are not in `svs.clients`, verified on
  all 11 bot servers in the 20 Aug sweep, which is precisely why H2 keeps them disjoint. The
  column is **Players**; `humans` is still forbidden, and so is any sum of the two figures. See
  `docs/rules.md` H1 as amended.
- **Ping with no colour bands or bars.** H11, `views.css:395-407`. A sort key, not a grade.
- **"Launched", never "Joined".** H12, `format.js:206-218`. Reveille started a process; the server
  decides admission and never reports it.
- **Bots on their own line, never summed; `capacity - clients` never computed.** H2 and H7,
  `format.js:37-46`. `sv_sharedbots` is unobservable and the ledger says so permanently. Do not let
  anyone "fix" this.
- **Absent rows kept rather than hidden, and the fold is not a filter.** H15. The disclosure states
  its count whether open or shut (`store.js:376-382`). Checked for drift; there is none.
- **`aria-disabled` instead of `disabled` on Check again.** `docs/ui.md` section 7 and
  `components.css:29-38`. Focus cannot be restored to a disabled element after a repaint.
  `docs/ux-standards.md` 6.6 independently agrees and goes further: do not take the contrast
  exemption either.
- **Arrow keys do not move rows while focus is inside a row button.** `servers.js:1007-1009`. A
  documented trade; F4's roving tabindex must preserve it.
- **`colspan` read off the header row rather than `COLUMNS.length`.** `servers.js:69-89`. Looks like
  over-engineering; it fixes a measured 1150 px layout bug where Chromium invents a phantom column.
- **A server that moved is not followed.** `app.js:453-465`. A shared query port is not proof of the
  same server.
- **No polling, no auto-refresh of the selected row.** F10's "deliberately does not do". Sending
  unsolicited traffic at third-party servers and moving figures under a player mid-decision are both
  worse than a stale reading with a timestamp.
- **No installed-content manager.** Friction F8, decided v2 on 22 Aug 2026.
- **`reveille-cli`'s `{:?}` output.** `plan.md:227` makes the CLI the headless pipeline driver, not
  a player surface. `main.rs:1391` is *not* covered by this — that one renders in the GUI.
- **`Compatible` renders no explanatory prose.** Section 9: "A ready server says nothing. Silence
  is the correct rendering of 'nothing to do'."
- **The `Mode` column shows `g_gametypestring` verbatim.** `format.js:71-86`. An invented
  FFA/OBJ/TDM would be a claim about a server Reveille cannot check.
- **`user-select: none` on chrome with `text` re-enabled on data.** `base.css:27` plus seven
  targeted overrides. Exactly right, and the thing most Tauri apps get wrong
  (`docs/ux-standards.md` 7.2).
- **Sort persisted across sessions, defaulting to clients descending.** `store.js:92`, `:242`.
  Matches the convention that "more is better" columns sort descending first.

The review also invoked none of the practices `docs/ux-standards.md` section 0 flags as thin or
overturned — no zebra striping, no skeleton screens, no designing for the F-pattern.

---

## Priority

Ranked by adoption impact rather than by ease.

| # | Item | Findings | Status |
|---|---|---|---|
| 1 | Packaging and signing | F1 | **Outstanding** — the one thing above every copy fix |
| 2 | Prerequisite copy and the glossary panel | F2, F9 | Done, 26 Aug 2026 |
| 3 | States rewritten as measurements | F3 | Done, 26 Aug 2026 |
| 4 | `role="grid"` and roving tabindex | F4, F11 | Done, 27 Aug 2026 |
| 5 | Error classification; keep stale rows on failure | F6 | Done, 27 Aug 2026 |
| 6 | One copy pass: "check", "Has people", contractions, labels | F5, F7, F20 | Done, 27 Aug 2026 |
| 7 | Needs column, ping threshold filter, search match count | F8, F13, F14, F15 | Done, 27 Aug 2026 |
| 8 | Context menu, `Ctrl+F`, `F6`, live-region cadence | F22, F23 | Done, 27 Aug 2026 |

**Reverted the same day, on the owner's review of the running app** (27 Aug 2026):

- The **Needs** column (F8) is gone again, and the narrow-window breakpoints are back at
  1200/1080/960. Mode outranks it. See open question 2.
- The detail pane's **Before you join** and **Maps** sections are gone. A compatible server now
  adds nothing to the pane at all, and the published rotation is not listed anywhere — see
  `docs/ui.md` §2.1 and §5. What survives is the state name and its explanation when there is one,
  the download size, and the candidate choices Reveille refuses to make on the player's behalf.
- **Clients** → **Players** and **Map now** → **Map** in the list; **Now** → **Map** in the pane.
  The first reverses a standing decision in this document and amends `docs/rules.md` H1; the
  measurement that allows it is recorded there.

Not in the table and still outstanding: F10, F12, F16, F17, F18, F19, F21.

---

## Open questions

1. **Is engine neutrality a rule or a default?** `docs/ui.md` section 3 says neither community
   engine gets a recommendation badge, but nothing in `docs/rules.md` forbids stating a
   *consequence*. Where is the line between "no recommendation" and "no help"?
2. ~~**Is the Needs column gone or parked?**~~ **Answered 27 Aug 2026: gone.** It was restored
   under F8 and removed again the same day by the project owner. The objection was not the price,
   which section 2.1 does pre-authorise, but the width — the column's 146px comes out of **Mode**,
   directly or through the breakpoint that drops Mode first, and Mode is what a player narrows by
   before anything else. The regression F8 named is accepted deliberately: the price is visible
   only after a row is selected. Any future answer to it has to be cheaper than a column charged
   to every row.
3. **Does the ten-minute criterion start at app launch or at the server list?** It decides whether
   F2 eats the whole budget. `plan.md:475` does not say where the stopwatch starts.
4. **Should first entry auto-sweep?** `enterServers` (`app.js:95-99`) fires around 190 probes the
   moment Continue is pressed, before the player has seen the interface.
5. **Is "check" load-bearing vocabulary?** F5 proposes retiring it for the sweep. If it is
   deliberate, a different disambiguation is needed instead.
6. **Does F16 match what the community actually does?**

---

## Implementation note

### Items 2 and 3, 26 Aug 2026

- **F2** — setup now states the prerequisite before any effort is spent, names the folder to look
  for, and distinguishes the game program from the game data on the not-found path.
- **F9** — a **What these words mean** panel, reachable from every view, defining the closed
  lexicon in one place.
- **F3** — the four state names are measurements rather than mood words, and each carries a
  persistent explanation instead of a `title`.

### Items 4 to 8, 27 Aug 2026

- **F4, F11** — the table carries `role="grid"` with the row and cell roles written out, and one
  roving tab stop. `↑`/`↓`/`Home`/`End` move between *every* row, so the absent block is reachable
  at all; `→`/`←` step into and along a row's controls, and `←` or `Escape` returns to the row.
  Verified live: one Tab from the selected row leaves the table entirely. The catalogue-lookup
  storm is closed by a ~220 ms settle on the selection rather than by a cancellation token — see
  the residual note below.
- **F6** — `BrowseFailureKind` classifies sweep failures in Rust beside the errors it names
  (`NoNetwork`, `MasterUnreachable`, `MasterUnreadable`, `Internal`), each rendering a cause and a
  remedy with the original message kept as detail. `main.rs`'s `{:?}` became `catalogue_reason`.
  A failed sweep no longer blanks the table: the rows from the last sweep that worked are kept and
  headed by a row saying which clock time they are from, and only when they were swept for the
  session still in force. The empty state gained the missing error branch, so the centre of the
  window and the corner can no longer contradict each other. Both paths were exercised against a
  real failure — a temporary unresolvable master host, reverted — rather than reasoned about.
- **F5, F7, F20** — *check* now means one thing: asking one server. The sweep is the **server
  list** throughout ("not in this list", "3 of 11 favorites in this list", "From the server list
  at 21:59"). `Join check` → **Before you join**; `Check this folder` → **Use this folder**; `Back
  to the check` → **Back to server details**; `checking archive` → **checking the file**;
  `Has people` → **Not empty**, identifier included; `Cannot join yet` → **Cannot join while this
  map is running**; the idiom "the server's call" is gone; the one player-facing negative
  contraction is gone.
- **F8, F13, F14, F15** — the **Needs** column is back as a price on `docs/ui.md` §2.1's terms,
  and did *not* displace Runs (see that section for why). Search matches the address as well as
  the name. The status bar says `61 of 114 shown` whenever a filter is narrowing. A **Ping under**
  ceiling gates what the sort could only reorder.
- **F22, F23** — a Reveille context menu on rows, opened by right-click or `Shift+F10`/Menu, with
  nothing in it that is not reachable another way; WebView2's own menu suppressed except over
  selectable text. `Ctrl+F` focuses search, `F6` cycles the three regions. Sweep progress is
  announced at quarters rather than per probe — five utterances instead of roughly two hundred.

Guarded by six new tests in `crates/reveille-app/src/main.rs`.

**Residual, deliberately not done.** F4 also asked for a cancellation token on `preview_join`.
The settle delay closes the storm it was for — holding `↓` through twenty rows now sends one
request — and a real token means changing `resolve_all_reporting`'s signature in `reveille-core`.
That is a core API change for a case the debounce already covers, so it is recorded here rather
than smuggled in.

Everything else in the priority table, F1 above all, is outstanding.
