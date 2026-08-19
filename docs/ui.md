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

So the **Needs** column states a price instead of passing a judgement:

| State | Cell | Colour |
|---|---|---|
| `Compatible` | *empty* | — |
| `NeedsMaps { count }` | `+ 7 maps` | none (default ink) |
| `NoSource { count }` | `7 maps unavailable` | `--bad-text`, the only coloured cell |
| `CantTell` | `not published` | `--faint`, italic |
| `CantTell`, current map absent | `+ 1 map` | none (default ink) |

Readiness is the **absence** of work, not an award. A ready server earns a blank cell.

The last row exists because a server that publishes no rotation is still running *something*. If
that map is not on disk the join is dropped on arrival, and one download fixes it — so it is priced
like any other work. The cell's `title` says the rest of the rotation remains unknown, so the figure
is never read as a complete bill.

Every cell carries a `title` with the full explanation, so the short label never has to carry the
whole meaning.

**The four canonical state names still exist and are unchanged** — `Compatible`, `Needs N maps`,
`No source`, `Can't tell`. They appear in the detail pane, under **Join check**, where the decision
is actually made and there is room to explain them. The list does not repeat them as badges.

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
│ ⌕ search   ☐ Has people  ☐ Hide unavailable  ▓▓▓░ 78/190 Stop│  toolbar
├───────────────────────────────────────┬──────────────────────┤
│ SERVER      CLIENTS  MAP NOW  RUNS  NEEDS │  detail pane      │
│ harzCore      40/64  dm/mohdm6  1.11      │  server facts     │
│ <[TFC]>        1/32  obj/bluts  1.11  +7 maps │  join check   │
│ [FORTE]       21/32  dm/mohdm6  1.11  not published │ maps     │
├───────────────────────────────────────┼──────────────────────┤
│ 106 of 190 answered · 108 bots · 84 not listed │ [Join]       │  status/actions
└───────────────────────────────────────┴──────────────────────┘
```

- **Setup** (`views/setup.js`) — shown *only* while no install is resolved. Not a welcome page: it
  answers "where is the game" and says how confidently, distinguishing a verified binary hash from
  a name-only match. Reachable again by clicking the install chip in the titlebar.
- **Servers** (`views/servers.js`) — where the session lives.
- **Detail pane** (`views/join.js`) — selection previews the join *in place*. No
  browser → join → back navigation; servers stay comparable; the list never disappears.
- Install progress and the launch outcome render inside the detail pane, not as separate screens.

## 4. Honesty rules, as UI rules

These are product contracts from `AGENTS.md` and `docs/engine-facts.md` §5, not style preferences.
Breaking one is a bug.

| Rule | How the interface satisfies it |
|---|---|
| Never call a client count "players" or "humans" | The column is **Clients**. Its tooltip says "Occupied slots reported by every server. Not verified as people." |
| Bots are disjoint from clients | Rendered on their own line as `+8 bots`, never summed into the client figure. The status bar says "counted separately". |
| Never imply free slots | Capacity appears only as a denominator (`21/32`). `capacity - clients` is never computed. |
| Never emit a boolean "can I join" | Four states, never a tick. `Compatible` is explained as "that is all Reveille can check — the server still decides whether you get in." |
| Never report a moh-db download as verified | Candidate rows show `tested` (the catalogue's own flag) and never "verified". Where a server publishes no checksum, the detail pane says an exact-file match cannot be confirmed. |
| Never auto-apply an ambiguous match | Choice radios start with **nothing selected**. The total excludes unresolved maps and the pane says how many still need a choice. |
| Say where files went | `used_home_fallback` prints the real `%APPDATA%\moh\main` path, not a euphemism. |
| A failure is a recorded non-result | Per-map install failures list individually; the pass is never abandoned. Unanswered endpoints are counted and broken down by reason in a dialog. |

## 5. The join gate

**Everything a server publishes about its content is checked — the rotation *and* the map it is
running now.** `classify_server` preflights `sv_maplist` plus `mapname`, deduplicated by `MapKey`,
because the two are not the same set: an admin can load a map directly, and a server can publish a
current map while publishing no rotation at all. Checking only the rotation missed the case that
matters most, and left the running map out of the shopping list so it could not even be fetched.

A server that published no rotation stays `Can't tell` even so. Its one checked map is real
evidence, but calling it `Compatible` would claim a rotation check that never happened. The detail
pane heads the section **Maps**, not Rotation, and says "No rotation published. Only the map running
now was checked."

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
| `Can't tell` | `Join without a rotation check` | `true` |
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

- The server list is a real `<table>` with `<caption>`, `scope="col"` and `aria-sort` — not a grid
  of buttons. Rows are `tabindex="0"` with `aria-selected`.
- **Keyboard**: `↑`/`↓`/`Home`/`End` move between rows, `Enter` and `Space` activate, `/` focuses
  search, `Escape` clears it, `F5` or `Ctrl+R` refreshes.
- `:focus-visible` shows a brass ring on every interactive element. Outlines are never removed.
- A `role="status" aria-live="polite"` region announces sweep progress and state changes.
  `role="alert"` is reserved for genuine errors.
- All motion respects `prefers-reduced-motion`.
- Progress is determinate wherever a total is known (`78/190`, byte counts) and indeterminate only
  during the master handshake, where nothing is known yet.
- Every long operation is cancellable: the sweep has a **Stop**, and selecting another server
  abandons the in-flight catalogue lookup.

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
  The detail pane shows the full string, where it has room.

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

- Say "clients", never "players".
- Say "did not answer", not "offline" — we know the former, not the latter.
- Say "not published" when a server published nothing. Silence is shown as silence and never
  upgraded to a tick.
- State consequences plainly: "you will be dropped when the rotation reaches this map" beats a
  warning icon.
- Never imply Reveille controls admission. Bans, capacity and ping limits are decided by the server
  at connect time, and the interface says so.
