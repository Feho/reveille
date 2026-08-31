// SPDX-License-Identifier: GPL-2.0-only

// The server list: toolbar, table, status bar.
//
// The table is a real <table> carrying `role="grid"`, so a screen reader gets a
// list of servers rather than a pile of controls, and a keyboard gets **one** tab
// stop for the whole thing. Every row used to be its own tab stop, which put the
// Join button roughly 260 Tab presses from the search box and made the arrow keys
// redundant; worse, focusing a row selected it, so arrowing down twenty rows fired
// twenty catalogue lookups at a third-party service (docs/design-review.md F4).
// A composite widget is one tab stop: the selected row holds `tabindex="0"` and
// every other row and in-row control holds `tabindex="-1"`.
//
// The list states no compatibility *verdict* — no badge, no green, no amber. See
// docs/ui.md §2.1: a status traffic light trains people to click only green, which
// would push roughly a quarter of the live population behind a colour that reads as
// a warning when it actually means "one click of downloads". What it does carry, in
// **Needs**, is the *price*: a countable quantity, colourless except for the one
// state a download cannot fix. The four canonical state names stay in the detail
// pane, where the decision is made and there is room to explain them.

import { el, fill, preserveFocus } from "../lib/dom.js";
import { openMenu } from "../lib/menu.js";
import {
  browseFailureText,
  gameType,
  launchedLabel,
  mapName,
  nonResultReason,
  occupancy,
  roundTrip,
  shortVersion,
} from "../lib/format.js";
import {
  clearHistory,
  favoriteAddresses,
  favorites,
  history,
  historyByAddress,
  toggleFavorite,
} from "../lib/bookmarks.js";
import {
  GAME_LABELS,
  PING_LIMITS,
  SCOPES,
  canRecheck,
  filtering,
  playableGames,
  savedEntries,
  saveFilters,
  scopedAbsent,
  scopedRows,
  state,
  update,
} from "../lib/store.js";

const COLUMNS = [
  // The star has no label: a column heading over one glyph reads as data. The cell's own
  // accessible name carries the meaning instead.
  { key: "star", label: "Favorite", sortable: false, className: "col-star", hideLabel: true },
  { key: "name", label: "Server", sortable: true, className: "col-name" },
  // "Players", not "Clients": `numplayers` is `SV_NumClients()` and bots are *not* in
  // `svs.clients` — measured live on all 11 bot servers (docs/plan.md, milestone 2). So the
  // figure counts human connections, and the bot count beside it is disjoint. It is still a
  // count of connections rather than of people at keyboards: a slot held by someone still
  // downloading is in it. The glossary says so; the column heading does not, because a heading
  // that hedges its own noun is read as a warning about the number rather than about the word.
  { key: "clients", label: "Players", sortable: true, numeric: true, className: "col-clients" },
  // "Map", not "Map now". Nothing else in the row is a map, and no other column is qualified by
  // when it was measured — the freshness of the whole row is stated once, in the detail pane.
  { key: "map", label: "Map", sortable: true, className: "col-map" },
  // The gametype the server publishes, in its own spelling — see `gameType` in lib/format.js
  // for why it is not shortened to FFA/OBJ/TDM.
  { key: "mode", label: "Mode", sortable: true, className: "col-mode" },
  // Ascending first: on a distance column the useful end is the small one, which is the
  // opposite of Players, where the busy servers are what a player is looking for.
  {
    key: "ping",
    label: "Ping",
    sortable: true,
    numeric: true,
    defaultDirection: "asc",
    className: "col-ping",
  },
  // A **Needs** column, pricing each server in maps to download, was added on 27 Aug 2026 and
  // removed the same day. It was not wrong — docs/ui.md §2.1 pre-authorised its return as a
  // *price*, and it did answer "which of these can I just join?" without a click each. It cost
  // more than it answered: a column on every row for a question asked about one row, drawn in a
  // width that had to come out of **Mode**, which is what a player filters the list by before
  // anything else. The price is in the detail pane, where the join is decided.
  { key: "runs", label: "Runs", sortable: false, className: "col-runs" },
];

/**
 * How many columns the table is actually drawing right now.
 *
 * Not `COLUMNS.length`: the narrow-window media queries in `styles/views.css` drop Runs, then
 * Ping, and a dropped column is gone from the table, not merely invisible. Every
 * `colspan` here has to agree with that or it runs off the end of the row — and a `colspan` that
 * overruns is not clipped. Chromium answers it by inventing the column the span asked for and
 * splitting the free width evenly between that phantom and **Server**, the only column with no
 * fixed width. That is what made the server name half its proper width in Favorites and History,
 * which have absent rows, and full width in All, which has none.
 *
 * Read from the header row rather than from a matching set of breakpoints in JavaScript, so the
 * media queries stay the one place the drop order is written down.
 */
function columnsShown(table) {
  const cells = [...table.tHead.rows[0].cells];
  const shown = cells.filter((cell) => cell.offsetParent !== null).length;
  // Nothing has an `offsetParent` until the table is in the document. The first paint after that
  // corrects it, and until then every column is drawn anyway.
  return shown || COLUMNS.length;
}

const SCOPE_LABELS = { all: "All", favorites: "Favorites", history: "History" };

const CAPTIONS = {
  all: "Servers answering now",
  favorites: "Starred servers, and whether they are in this list",
  history: "Servers Reveille launched the game for, most recent first",
};

/**
 * Leaving a scope clears the selection, because the selected server may not be in the one being
 * entered — and a detail pane describing a server no longer in the list is the stale reading H12
 * exists to prevent.
 *
 * History arrives ordered by when the game was last launched, which is the only ordering that
 * makes a history a history. No column header owns that key, so no arrow is drawn. Leaving
 * History restores an ordinary column sort rather than carrying a key nothing can show.
 */
function selectScope(scope) {
  if (state.scope === scope) return;
  update((next) => {
    const wasHistory = next.scope === "history";
    next.scope = scope;
    next.selected = null;
    next.preview = null;
    if (scope === "history") next.sort = { column: "launched", direction: "desc" };
    else if (wasHistory) next.sort = { column: "clients", direction: "desc" };
    saveFilters();
  });
}

export function serversView({ onRefresh, onCancel, onSelect, onShowNonResults, onCheck, onGame }) {
  const search = el("input", {
    id: "server-search",
    type: "search",
    autocomplete: "off",
    spellcheck: false,
    placeholder: "Search server names",
    "aria-label": "Search server names",
    oninput: (event) => update((next) => (next.filters.query = event.target.value)),
  });

  // The toolbar is built once and updated in place. Rebuilding it on every
  // render would detach the search input mid-keystroke and drop the caret,
  // which makes the field accept exactly one character.

  // Which population the table lists. Three exclusive buttons rather than tabs: there is still
  // one table, one set of columns and one selection — only the rows it draws from change.
  const scopeButtons = SCOPES.map((scope) =>
    el(
      "button",
      {
        type: "button",
        className: "scope__option",
        "aria-pressed": "false",
        dataset: { focusKey: `scope-${scope}`, scope },
        onclick: () => selectScope(scope),
      },
      el("span", { className: "scope__label" }, SCOPE_LABELS[scope]),
      el("span", { className: "scope__count data" }),
    ),
  );
  const scopeGroup = el(
    "div",
    { className: "scope", role: "group", "aria-label": "Which servers to list" },
    scopeButtons,
  );

  // Which of the three games this session is browsing. It sits before the scope buttons because
  // it decides which population they draw from: Allied Assault, Spearhead and Breakthrough
  // register with the master separately, so this is a different list, not a filter over one.
  // Hidden entirely on an install that has only one of them — a control with one option is noise.
  const gameSelect = el("select", {
    id: "game-select",
    dataset: { focusKey: "game-select" },
    onchange: (event) => onGame(event.target.value),
  });
  const gameField = el(
    "label",
    { className: "toolbar__game", for: "game-select" },
    el("span", { className: "label" }, "Game"),
    gameSelect,
  );

  // "Not empty", never "Has people". The figure this gates on is occupied slots, and a slot counts
  // from `connect` onwards — so it counts a player still downloading or sitting at a menu, and a
  // stale connection the server has not timed out yet. Enough to say a server is not empty; not
  // enough to promise people (rule H1, docs/design-review.md F7).
  const notEmpty = toggle("Not empty", () =>
    update((next) => {
      next.filters.notEmpty = !next.filters.notEmpty;
      saveFilters();
    }),
  );

  // The ping gate. Sorting by a column is not filtering on it: sorting by players surfaces full
  // servers on the far side of the world, and shipping the sort without the gate is a documented
  // failure across several modern browsers (docs/design-review.md F15).
  const pingSelect = el("select", {
    id: "ping-limit",
    dataset: { focusKey: "ping-limit" },
    onchange: (event) =>
      update((next) => {
        const value = event.target.value;
        next.filters.maxPing = value === "" ? null : Number(value);
        saveFilters();
      }),
  });
  fill(
    pingSelect,
    ...PING_LIMITS.map((limit) =>
      el("option", { value: limit === null ? "" : String(limit) }, limit === null ? "Any" : `${limit} ms`),
    ),
  );
  const pingField = el(
    "label",
    { className: "toolbar__ping", for: "ping-limit" },
    // "Round trip", not "ping under": the figure this gates is one UDP sample taken during the
    // sweep, which is what the Ping column says too. Naming the control after the column keeps the
    // two the same claim.
    el("span", { className: "label" }, "Ping under"),
    pingSelect,
  );
  // Both browse controls are built once and shown or hidden, never rebuilt. A
  // sweep notifies several times a second, and replacing a button between its
  // mousedown and mouseup swallows the click — which is why Stop appeared dead.
  const meterFill = el("span", { className: "meter__fill" });
  const meter = el(
    "div",
    { className: "meter", role: "progressbar", "aria-label": "Getting the server list" },
    meterFill,
  );
  const meterCount = el("span", { className: "quiet data" });
  const stopButton = el(
    "button",
    { type: "button", className: "btn btn--sm", dataset: { focusKey: "browse-stop" }, onclick: onCancel },
    "Stop",
  );
  const progress = el("div", { className: "toolbar__progress" }, meter, meterCount, stopButton);
  const refresh = el(
    "button",
    {
      type: "button",
      className: "btn btn--primary",
      dataset: { focusKey: "browse-refresh" },
      onclick: onRefresh,
    },
    "Find servers",
  );
  const actionSlot = el("div", { className: "toolbar__action" }, progress, refresh);
  const toolbar = el(
    "div",
    { className: "toolbar" },
    gameField,
    scopeGroup,
    el(
      "label",
      { className: "field toolbar__search", for: "server-search" },
      el("span", { className: "field__icon", "aria-hidden": "true" }, "⌕"),
      search,
    ),
    notEmpty,
    pingField,
    el("span", { className: "toolbar__spacer" }),
    actionSlot,
  );
  const tbody = el("tbody", {
    onkeydown: (event) => {
      // Shift+F10 and the Menu key are how a keyboard opens a context menu on Windows. Without
      // them the menu below would be a mouse-only feature, which is what makes a context menu an
      // accessibility problem rather than a convention.
      if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
        onRowContextMenu(event, onSelect, onCheck);
        return;
      }
      onRowKey(event, onSelect);
    },
    oncontextmenu: (event) => onRowContextMenu(event, onSelect, onCheck),
  });
  // Header cells are built once and their sort state written in place. Rebuilding them left the
  // arrow and the highlight frozen on whichever column was sorted when the view was created.
  const headers = COLUMNS.map(headerCell);
  // `role="grid"`, with the row and cell roles written out rather than left to the HTML-AAM
  // mapping. Two things depend on it. `aria-selected` is not supported on `row` inside the plain
  // `table` role, so the app's primary interaction — which server is selected — was invisible to
  // assistive technology (docs/design-review.md F11). And a grid is a **composite** widget, which
  // is what licenses the single tab stop below.
  const table = el(
    "table",
    { className: "servers", role: "grid", "aria-label": "Servers" },
    el("caption", { className: "sr-only" }),
    el(
      "thead",
      { role: "rowgroup" },
      el("tr", { role: "row" }, headers.map((header) => header.th)),
    ),
    tbody,
  );
  tbody.setAttribute("role", "rowgroup");
  const caption = table.querySelector("caption");
  const listPane = el("div", { className: "list-pane" }, table);
  const statusbar = el("div", { className: "statusbar" });
  const live = el("p", { className: "sr-only", role: "status", "aria-live": "polite" });

  let lastSignature = null;
  let lastPainted = 0;
  let pending = null;
  let lastColumns = COLUMNS.length;

  // Nothing else in the app watches the window size. Without this, dragging the edge across a
  // breakpoint leaves every absent row holding the colspan it was painted with, and the server
  // name stays at the width that colspan produced until something unrelated forces a repaint.
  // Only an actual change in the column count repaints, so a drag that crosses none costs one
  // layout read per event.
  window.addEventListener("resize", () => {
    if (columnsShown(table) !== lastColumns) update(() => {});
  });

  // Rebuilt only when the detected products change, which is once per install: replacing the
  // options on every render would drop the open dropdown mid-choice.
  let lastGames = null;
  const paintGame = () => {
    const games = playableGames(state.install);
    gameField.classList.toggle("hidden", games.length < 2);
    const signature = games.join("|");
    if (signature !== lastGames) {
      lastGames = signature;
      fill(
        gameSelect,
        ...games.map((game) => el("option", { value: game }, GAME_LABELS[game] ?? game)),
      );
    }
    gameSelect.value = state.game;
    // Switching mid-sweep would leave probes in flight for the game just left, and a download
    // cannot be abandoned half-written at all.
    gameSelect.disabled = state.browse.running || state.joining;
  };

  const paintScope = () => {
    const counts = { all: state.servers.length, favorites: favorites().length, history: history().length };
    for (const button of scopeButtons) {
      const { scope } = button.dataset;
      button.setAttribute("aria-pressed", state.scope === scope ? "true" : "false");
      const count = counts[scope];
      button.querySelector(".scope__count").textContent = count ? String(count) : "";
    }
  };

  const paintHeaders = () => {
    for (const { column, th, arrow } of headers) {
      const active = column.sortable && state.sort.column === column.key;
      const ascending = state.sort.direction === "asc";
      if (active) th.setAttribute("aria-sort", ascending ? "ascending" : "descending");
      else th.removeAttribute("aria-sort");
      if (arrow) arrow.textContent = active ? (ascending ? "▲" : "▼") : "";
    }
  };

  const paintAction = () => {
    const running = state.browse.running;
    progress.classList.toggle("hidden", !running);
    refresh.classList.toggle("hidden", running);
    refresh.textContent = state.servers.length ? "Refresh" : "Find servers";
    if (!running) return;
    // The sweep spawns no further probes once stopped, but the ones already in
    // flight still have to time out. Say so rather than leaving Stop looking inert.
    stopButton.disabled = state.browse.stopping;
    stopButton.textContent = state.browse.stopping ? "Stopping…" : "Stop";
    const { probed, inspected } = state.browse;
    const known = inspected > 0;
    meter.classList.toggle("meter--indeterminate", !known);
    meterFill.style.width = known
      ? `${Math.min(100, Math.round((probed / inspected) * 100))}%`
      : "";
    meterCount.textContent = known ? `${probed}/${inspected}` : "contacting master";
    if (known) {
      meter.setAttribute("aria-valuenow", String(probed));
      meter.setAttribute("aria-valuemin", "0");
      meter.setAttribute("aria-valuemax", String(inspected));
    }
  };

  /**
   * The selection, and the one tab stop that goes with it.
   *
   * Roving tabindex: exactly one row in the grid is tabbable, and it is the selected one — or the
   * first row when nothing is selected yet, so Tab into an untouched list lands somewhere sensible
   * rather than nowhere. Everything else, rows and the controls inside them, is `-1` and reached
   * with the arrow keys. Written in place on every render for the same reason the stars are: the
   * selection changes without changing which rows exist, so `paintRows` correctly skips.
  */
  const syncSelection = () => {
    const gridRows = [...tbody.children];
    const rows = gridRows.filter((tr) => tr.dataset.address);
    const selected = rows.find((tr) => tr.dataset.address === state.selected);
    // A saved scope can contain only the disclosure and absent rows. The grid still needs one
    // entry point: once focus lands on that non-live row, the normal arrow model reaches its
    // disclosure and Check controls (docs/ui.md §7).
    const tabbable = selected ?? rows[0] ?? gridRows[0] ?? null;
    for (const tr of gridRows) {
      if (tr.dataset.address) {
        tr.setAttribute("aria-selected", tr === selected ? "true" : "false");
      }
      // Absent and disclosure rows are not selectable, but they are still rows in the grid and
      // still hold controls, so they take the same treatment: never a second tab stop.
      tr.tabIndex = tr === tabbable ? 0 : -1;
      for (const control of tr.querySelectorAll("button, input, select, a[href]")) {
        control.tabIndex = -1;
      }
    }
  };

  /**
   * Write every star's state in place, like the selection.
   *
   * Starring changes nothing the row signature covers — same rows, same order — so `paintRows`
   * correctly skips, and without this the glyph stayed as it was until something else forced a
   * repaint. It converges every path that can star a server: the row's own button, the detail
   * pane's, and the `F` key.
   */
  const syncStars = () => {
    const starred = favoriteAddresses();
    for (const tr of tbody.children) {
      const address = tr.dataset.address ?? tr.dataset.remembered;
      const button = address && tr.querySelector(".star");
      if (!button) continue;
      const on = starred.has(address);
      button.setAttribute("aria-pressed", on ? "true" : "false");
      button.title = on ? "Remove from favorites" : "Add to favorites";
      button.textContent = on ? "★" : "☆";
    }
  };

  const paintRows = () => {
    lastPainted = performance.now();
    const items = scopedRows();
    lastSignature = signature(items);
    lastColumns = columnsShown(table);
    // Read the starred set once. Asking per row would parse the store a hundred-odd times a paint.
    const starred = favoriteAddresses();
    const launches = state.scope === "history" ? historyByAddress() : null;
    const build = (item) => {
      if (item.kind === "live") return row(item.row, starred, launches, onSelect);
      if (item.kind === "disclosure") return disclosureRow(item.count, lastColumns);
      return absentRow(item.entry, starred, launches, lastColumns, onCheck, onGame);
    };
    // Opening the absent block repaints the table the button that opened it lives in, and a
    // keyboard would be left on nothing. The disclosure carries a focus key so it comes back.
    preserveFocus(tbody, () =>
      fill(
        tbody,
        // A list left standing after a sweep failed says so in the row above it, not only in the
        // corner. What is on screen is a past reading, and the one thing it must never do is read
        // as a current one (docs/ux-standards.md §4.5).
        state.staleAt ? staleRow(lastColumns) : null,
        items.length ? items.map(build) : emptyRow(lastColumns),
      ),
    );
    // `aria-rowcount` counts the data rows, so a screen reader can say "row 4 of 130" whether or
    // not the stale banner is drawn.
    table.setAttribute("aria-rowcount", String(items.length));
    syncSelection();
  };

  const render = () => {
    notEmpty.setAttribute("aria-pressed", state.filters.notEmpty ? "true" : "false");
    pingSelect.value = state.filters.maxPing === null ? "" : String(state.filters.maxPing);
    if (search.value !== state.filters.query) search.value = state.filters.query;
    paintGame();
    paintScope();
    caption.textContent = CAPTIONS[state.scope];
    paintHeaders();
    paintAction();
    fill(statusbar, ...statusbarContents(onShowNonResults, onCheck));
    announce(live);

    const next = signature(scopedRows());
    // The column count is not in the signature — it is not a property of the rows — but crossing a
    // breakpoint changes every colspan the last paint wrote, so it has to force one too.
    if (next === lastSignature && columnsShown(table) === lastColumns) {
      syncSelection();
      syncStars();
      return;
    }
    // A sweep emits one event per probed endpoint. Repainting ~130 rows on each
    // would fight the sweep for the main thread, so coalesce to ~4 Hz while it
    // runs, then paint immediately once it stops.
    const since = performance.now() - lastPainted;
    if (state.browse.running && since < 250) {
      pending ??= setTimeout(() => {
        pending = null;
        paintRows();
      }, 250 - since);
      return;
    }
    if (pending !== null) {
      clearTimeout(pending);
      pending = null;
    }
    paintRows();
  };

  return {
    render,
    toolbar,
    listPane,
    statusbar,
    live,
    focusSearch: () => search.focus(),
    // The grid's one tab stop, wherever `syncSelection` put it.
    focusFirstRow: () => tabbableRow(tbody)?.focus(),
  };
}

/** The row currently holding the grid's tab stop. */
function tabbableRow(tbody) {
  return tbody.querySelector('tr[tabindex="0"]') ?? tbody.querySelector("tr[data-address]");
}

/** One header cell, plus the nodes whose sort state is written on every render. */
function headerCell(column) {
  // The sort buttons stay in the ordinary tab order rather than joining the roving tabindex.
  // There are five of them, not two hundred, and folding them into the grid's arrow navigation
  // would make the one control that reorders the list reachable only by first entering the rows
  // it reorders. The 260-tab-stop problem F4 names is the body, and that is what was fixed.
  const attrs = {
    role: "columnheader",
    scope: "col",
    className: [column.numeric ? "num" : null, column.className].filter(Boolean).join(" ") || null,
  };
  if (column.hideLabel) {
    return {
      column,
      th: el("th", attrs, el("span", { className: "sr-only" }, column.label)),
      arrow: null,
    };
  }
  if (!column.sortable) return { column, th: el("th", attrs, column.label), arrow: null };

  const arrow = el("span", { className: "sort-arrow" });
  const th = el(
    "th",
    attrs,
    el(
      "button",
      {
        type: "button",
        onclick: () =>
          update((next) => {
            if (next.sort.column === column.key) {
              next.sort.direction = next.sort.direction === "asc" ? "desc" : "asc";
            } else {
              next.sort = {
                column: column.key,
                direction: column.defaultDirection ?? (column.numeric ? "desc" : "asc"),
              };
            }
            saveFilters();
          }),
      },
      column.label,
      arrow,
    ),
  );
  return { column, th, arrow };
}

function toggle(label, onclick) {
  return el(
    "button",
    {
      type: "button",
      className: "toggle",
      "aria-pressed": "false",
      dataset: { focusKey: `toggle-${label}` },
      onclick,
    },
    el("span", { className: "toggle__box", "aria-hidden": "true" }, "✓"),
    label,
  );
}

/**
 * The star. A real button, so it is reachable by keyboard and announces its own state; the click
 * must not fall through to the row, or starring would also change the selection.
 *
 * Takes a live row or a remembered entry, because a favorite can be unstarred from a row this
 * sweep did not return.
 */
function starCell(subject, address, hostname, starred) {
  const on = starred.has(address);
  const name = hostname || address;
  return el(
    "td",
    { role: "gridcell", className: "col-star" },
    el(
      "button",
      {
        type: "button",
        className: "star",
        // The grid owns the tab order; `syncSelection` rewrites this on every render anyway.
        tabIndex: -1,
        "aria-pressed": on ? "true" : "false",
        "aria-label": `Favorite ${name}`,
        title: on ? "Remove from favorites" : "Add to favorites",
        onclick: (event) => {
          event.stopPropagation();
          toggleFavorite(subject);
          update(() => {});
        },
      },
      on ? "★" : "☆",
    ),
  );
}

function row(item, starred, launches, onSelect) {
  const { clients, bots, capacity } = occupancy(item.server);
  const ping = roundTrip(item.server);
  const mode = gameType(item.server);
  const launched = launches ? launchedLabel(launches.get(item.address)) : null;
  const choose = () => {
    if (state.selected !== item.address) onSelect(item.address);
  };
  return el(
    "tr",
    {
      role: "row",
      // The address doubles as the focus key: rows are rebuilt on every paint, and without it a
      // keyboard player loses the caret whenever a probe lands.
      dataset: { address: item.address, focusKey: item.address },
      // Set by `syncSelection`, which owns the single tab stop. Never 0 here.
      tabIndex: -1,
      "aria-selected": "false",
      onclick: choose,
      // Selection follows focus, which is the grid convention for a single-select list and what
      // makes the arrow keys useful. It is no longer a network storm: focus now moves only on a
      // deliberate arrow press, and the catalogue lookup behind it is debounced (app.js `select`).
      onfocus: choose,
    },
    starCell(item, item.address, item.server.hostname, starred),
    el(
      "td",
      { role: "gridcell", className: "col-name" },
      el(
        "span",
        { className: "server-name", title: item.server.hostname },
        item.server.hostname || "(unnamed server)",
      ),
      el("span", { className: "server-address" }, item.address),
      // "Launched", never "joined": Reveille started the game and saw it start. Whether the
      // server let the player in is decided at connect time and never observed (H12).
      launched && el("span", { className: "history-line" }, launched),
    ),
    el(
      "td",
      { role: "gridcell", className: "num col-clients" },
      el(
        "span",
        { className: "occupancy" },
        el("span", { className: "occupancy__clients" }, clients === null ? "—" : String(clients)),
        capacity !== null && el("span", { className: "occupancy__capacity" }, `/${capacity}`),
      ),
      // Bots are a disjoint quantity and are never folded into the client count.
      bots !== null && el("span", { className: "occupancy__bots" }, `+${bots} bots`),
    ),
    el(
      "td",
      { role: "gridcell", className: "col-map" },
      el(
        "span",
        { className: "map-cell" },
        item.server.current_map ? mapName(item.server.current_map) : "—",
      ),
    ),
    el(
      "td",
      { role: "gridcell", className: "col-mode" },
      el("span", { className: "mode-cell", title: mode.title }, mode.text),
    ),
    el(
      "td",
      { role: "gridcell", className: "num col-ping" },
      el("span", { className: "ping-cell", title: ping.title }, ping.text),
    ),
    el(
      "td",
      { role: "gridcell", className: "col-runs" },
      el(
        "span",
        { className: "runs-cell", title: item.server.version ?? "" },
        shortVersion(item.server),
      ),
    ),
  );
}

/** The scope's own word for what it holds, singular or plural. */
const SAVED_NOUNS = {
  favorites: ["favorite", "favorites"],
  history: ["launched server", "launched servers"],
};

function savedNoun(count = 0) {
  const [one, many] = SAVED_NOUNS[state.scope] ?? ["server", "servers"];
  return count === 1 ? one : many;
}

/**
 * The head of the absent block: how many remembered entries this list does not hold, and the
 * control that folds them out of the way.
 *
 * Absent entries used to sit open at the foot of Favorites and History. On a folder with more
 * than one game that is where most of them live for ever — Allied Assault, Spearhead and
 * Breakthrough register with the master separately, so a server starred under one of them is
 * never in another's list and its Check button can only ever find the same thing. Twenty rows of
 * "not in this list" under three that answered reads as a broken list.
 *
 * Shut, this is not a filter with an invisible effect (docs/ui.md §2.1, rule H15): the count is on
 * screen, in the row where the entries would have been, and one click brings them back. Nothing is
 * classified, guessed or dropped — the criterion is the same one the rows themselves state, which
 * this list demonstrably does not hold.
 */
function disclosureRow(count, columns) {
  const open = state.showAbsent;
  // Naming the game earns its place only where the folder has more than one: it is why most of
  // these entries can never answer. On a single-game folder it is noise, and the game select is
  // hidden there for the same reason.
  const check =
    playableGames(state.install).length > 1
      ? `this ${GAME_LABELS[state.game] ?? state.game} list`
      : "this list";
  return el(
    "tr",
    // No `data-address`: there is nothing here to select or preview.
    { role: "row", className: "row-disclosure", tabIndex: -1 },
    el(
      "td",
      { role: "gridcell", colspan: String(columns) },
      el(
        "button",
        {
          type: "button",
          className: "disclosure",
          tabIndex: -1,
          "aria-expanded": open ? "true" : "false",
          dataset: { focusKey: "absent-disclosure" },
          title:
            "These were not in the list the master server returned, so they were never asked. Servers saved under another game always end up here.",
          onclick: () =>
            update((next) => {
              next.showAbsent = !next.showAbsent;
              saveFilters();
            }),
        },
        el("span", { className: "disclosure__caret", "aria-hidden": "true" }, open ? "▾" : "▸"),
        `${count} ${savedNoun(count)} not in ${check}`,
      ),
    ),
  );
}

/**
 * A remembered server the current check did not return.
 *
 * It keeps its star and its name, and **nothing else** — no client count, no map, no round trip.
 * Those were true of a past moment, and drawn in these columns they would read as now (H12). What
 * the row offers instead is the one thing that can change that: check this server on its own.
 *
 * The name is the one remembered thing left on the row, and it is not labelled as such: the row
 * says "not in this list" across the columns where every figure would have been, so there is
 * nothing here a reader could take for a current measurement. The italic `--remembered` name
 * carries the rest.
 *
 * The wording is "not in this list", not "offline". The sweep asks the master for a list and
 * probes what comes back; a server missing from that list was never asked, which is a different
 * fact from not answering. Only a check that actually failed may say the server did not answer.
 */
function absentRow(entry, starred, launches, columns, onCheck, onGame) {
  const check = state.checks.get(entry.address);
  const launched = launches ? launchedLabel(launches.get(entry.address)) : null;
  return el(
    "tr",
    // Deliberately no `data-address`: that attribute marks a selectable row, and there is
    // nothing here to select or preview until the server answers.
    {
      role: "row",
      className: "row-absent",
      tabIndex: -1,
      dataset: { remembered: entry.address, focusKey: `absent-${entry.address}` },
    },
    starCell(entry, entry.address, entry.hostname, starred),
    el(
      "td",
      { role: "gridcell", className: "col-name" },
      el(
        "span",
        { className: "server-name server-name--remembered", title: entry.hostname },
        entry.hostname || "(unnamed server)",
      ),
      el("span", { className: "server-address" }, entry.address),
      launched && el("span", { className: "history-line" }, launched),
    ),
    el(
      "td",
      // The star and the name keep their own cells; the note and its button take everything left.
      { role: "gridcell", colspan: String(columns - 2) },
      el("span", { className: "absent-note" }, absentNote(check)),
      absentAction(entry, check, onCheck, onGame),
    ),
  );
}

/**
 * The one control an absent row offers, which is not always Check.
 *
 * A bookmark stores an address, so it outlives the game it was saved under. Once a check has found
 * that this server runs another of the three, checking it again can only find the same thing: what
 * moves the player forward is switching to that game — when this folder has it. When it does not,
 * there is no action to offer and none is drawn, because a button that cannot work is worse than
 * no button.
 */
function absentAction(entry, check, onCheck, onGame) {
  const other = check?.otherGame;
  if (other) {
    if (!playableGames(state.install).includes(other)) return false;
    return el(
      "button",
      {
        type: "button",
        className: "btn btn--sm",
        disabled: state.browse.running || state.joining,
        onclick: (event) => {
          event.stopPropagation();
          onGame(other);
        },
      },
      `Switch to ${GAME_LABELS[other] ?? other}`,
    );
  }
  const checking = check?.status === "checking";
  return el(
    "button",
    {
      type: "button",
      className: "btn btn--sm",
      // `aria-disabled` rather than `disabled`, as in the detail pane: this button goes busy the
      // moment it is pressed, and the repaint cannot return focus to a disabled element.
      "aria-disabled": checking ? "true" : null,
      onclick: (event) => {
        event.stopPropagation();
        if (checking) return;
        onCheck(entry);
      },
    },
    checking ? "Checking…" : "Check",
  );
}

/** What is actually known about an absent server, which before a check is very little. */
function absentNote(check) {
  if (check?.status === "checking") return "checking";
  // A command that never ran is not a server that did not answer. Only the second may be reported
  // as a fact about the server (H12).
  if (check?.status === "failed") {
    return el("span", { title: check.error }, "the check could not run");
  }
  if (check?.movedTo) {
    return `answers at ${check.movedTo} now`;
  }
  // It answered — it is simply another of the three games, which this session's client cannot
  // join. What to do about that is said in the row, not in a tooltip a keyboard cannot reach: the
  // action beside it switches games, and when this folder cannot run that game the sentence says
  // so instead, because then there is nothing to switch to.
  if (check?.otherGame) {
    const name = GAME_LABELS[check.otherGame] ?? check.otherGame;
    return playableGames(state.install).includes(check.otherGame)
      ? `runs ${name}`
      : `runs ${name}, which is not in this game folder`;
  }
  if (check?.nonResult) {
    return el(
      "span",
      { title: `This server ${nonResultReason(check.nonResult)}.` },
      "did not answer",
    );
  }
  return el(
    "span",
    {
      title:
        "This server was not in the list the master server returned, so it was never asked. Check it on its own to find out.",
    },
    "not in this list",
  );
}

/**
 * The row that says the list on screen is not this session's answer.
 *
 * It sits above the rows rather than replacing them. A sweep that could not reach the master used
 * to blank the table, so the centre of the window read "Nothing has been checked yet" while the
 * corner held an error about a check that had just run — the two contradicting each other with no
 * next action in either (docs/design-review.md F6). Keeping the rows and marking them is both
 * more honest and more useful: those servers were real, they simply have not been re-asked.
 */
function staleRow(columns) {
  return el(
    "tr",
    { role: "row", className: "row-stale", tabIndex: -1 },
    el(
      "td",
      { role: "gridcell", colspan: String(columns) },
      el("span", { className: "row-stale__mark", "aria-hidden": "true" }, "⌛"),
      `This list is from ${state.staleAt}. The check just now did not finish, so these figures have not been re-asked.`,
    ),
  );
}

function emptyRow(columns) {
  let body;
  // A scope with nothing saved and a scope whose entries are all filtered out are different
  // problems, and only one of them is fixed by clearing the search box.
  if (state.scope === "favorites" && favorites().length === 0) {
    body = [el("h3", null, "No favorites yet"), el("p", null, "Star a server to keep it here.")];
  } else if (state.scope === "history" && history().length === 0) {
    body = [
      el("h3", null, "Nothing launched yet"),
      el("p", null, "A server appears here once Reveille has started the game for it."),
    ];
  } else if (state.scope !== "all") {
    body = [
      el("h3", null, "Nothing matches"),
      el("p", null, "No saved server matches the current search and filters."),
    ];
  } else if (state.browse.running) {
    body = [
      el("h3", null, "Getting the server list"),
      el("p", null, "Rows appear as each server answers."),
    ];
  } else if (state.browse.error) {
    // The centre of the window and the corner now say the same thing. Before this branch existed
    // the table claimed nothing had ever been checked while the status bar carried a raw error
    // from the check that had just failed (docs/design-review.md F6).
    const failure = browseFailureText(state.browse.error);
    body = [
      el("h3", null, failure.title),
      el("p", null, failure.remedy ?? failure.detail),
      el("p", { className: "quiet" }, "Nothing was reached, so nothing is listed."),
    ];
  } else if (state.servers.length) {
    body = [
      el("h3", null, "Nothing matches"),
      el(
        "p",
        null,
        filtering()
          ? "No server matches the current search and filters."
          : "The list is empty for this view.",
      ),
    ];
  } else {
    body = [
      el("h3", null, "No servers yet"),
      el("p", null, "Press Find servers to ask the master server who is online."),
    ];
  }
  return el(
    "tr",
    { role: "row", tabIndex: -1 },
    el(
      "td",
      { role: "gridcell", colspan: String(columns) },
      el("div", { className: "placeholder" }, body),
    ),
  );
}

/**
 * Why the sweep failed, in the player's words, with the original message kept as detail.
 *
 * Every browse failure used to reach the status bar as `error.to_string()` — "GameSpy encryption
 * key is empty", "master reply body has 42 bytes; expected a multiple of 6" — as the *entire*
 * status bar. The cause is now classified in Rust beside the errors it names, exactly as engine
 * failures already were, and the shell chooses the sentence (docs/design-review.md F6).
 */
function failureStatus(failure) {
  const { title, remedy, detail } = browseFailureText(failure);
  return [
    el("span", { className: "error" }, title),
    remedy ? el("span", null, remedy) : null,
    el("span", { className: "statusbar__spacer" }),
    // Kept, not hidden: it is what a bug report needs. It is simply no longer the whole message.
    detail ? el("span", { className: "quiet", title: detail }, "details") : null,
  ];
}

function statusbarContents(onShowNonResults, onCheck) {
  const { summary, browse } = state;
  if (browse.error) return failureStatus(browse.error);
  if (state.scope !== "all") return scopedStatusbar(onCheck);
  if (!summary && !browse.running) return [el("span", null, "Not checked yet")];

  const answered = summary ? summary.getstatus_reachable : browse.answered;
  const registered = summary ? summary.registered : browse.registered;
  const skipped = summary ? summary.non_results : browse.nonResults;
  return [
    el("span", null, el("strong", null, String(answered)), ` of ${registered} answered`),
    // How much of the list the search box and the toolbar are hiding. Without it "Nothing matches"
    // is the only feedback a filter ever gives, and it arrives only once the filter has hidden
    // everything (docs/ux-standards.md §2, docs/design-review.md F14).
    filtering() &&
      state.servers.length > 0 &&
      el(
        "span",
        null,
        el("strong", null, String(scopedRows().length)),
        ` of ${state.servers.length} shown`,
      ),
    summary &&
      el(
        "span",
        {
          title:
            "Occupied slots reported by every server. Bots are not in this figure; a slot held by someone still connecting is.",
        },
        el("strong", null, String(summary.clients_reported)),
        " players reported",
      ),
    summary &&
      summary.bots_reported > 0 &&
      el(
        "span",
        null,
        el("strong", null, String(summary.bots_reported)),
        " bots, counted separately",
      ),
    skipped > 0 &&
      el(
        "button",
        { type: "button", onclick: onShowNonResults },
        `${skipped} registered but not listed`,
      ),
    el("span", { className: "statusbar__spacer" }),
    browse.cancelled && el("span", null, "stopped early"),
    browse.completedAt && el("span", null, browse.completedAt),
  ];
}

/**
 * The status bar for a saved scope. It counts what is saved and how much of it this check
 * returned, rather than repeating the sweep's totals, which are about a different population.
 *
 * This count is taken against the whole saved set, search box or no search box: it is a statement
 * about the check, not about the current query. The disclosure row counts what it is actually
 * hiding, which is the filtered set, so the two answer different questions and say so.
 */
function scopedStatusbar(onCheck) {
  const saved = savedEntries();
  const present = new Set(state.servers.map((row) => row.address));
  const missing = saved.filter((entry) => !present.has(entry.address));
  // The button acts on rows, so it takes the rows the block is showing.
  const shown = scopedAbsent();

  return [
    // "0 of 0" is noise; the empty state in the table already says what is going on.
    saved.length > 0 &&
      el(
        "span",
        null,
        el("strong", null, String(saved.length - missing.length)),
        ` of ${saved.length} ${savedNoun(saved.length)} in this list`,
      ),
    // No separate "N of M shown" here: the line above already counts the saved set against this
    // list, and a second ratio beside it would be two different denominators in one status bar.
    // Offered only while the absent block is open. Shut, the whole of its effect — absent rows
    // changing what they say — would happen where nobody could see it, which is the one thing a
    // control in this interface may not do (docs/ui.md §2.1).
    state.showAbsent &&
      shown.length > 0 &&
      el(
        "button",
        {
          type: "button",
          title: "Ask each of these servers directly, one request each.",
          onclick: () => onCheck(shown),
        },
        `Check the other ${shown.length}`,
      ),
    el("span", { className: "statusbar__spacer" }),
    state.scope === "history" && saved.length > 0 && clearHistoryButton(),
  ];
}

/**
 * Clearing history is not undoable, so it asks twice — a second click rather than a dialog,
 * which is one interruption fewer for an action nobody reaches by accident.
 */
function clearHistoryButton() {
  const button = el(
    "button",
    {
      type: "button",
      onclick: () => {
        if (button.dataset.armed !== "true") {
          button.dataset.armed = "true";
          button.textContent = "Click again to clear";
          return;
        }
        clearHistory();
        update((next) => {
          next.selected = null;
          next.preview = null;
        });
      },
    },
    "Clear history",
  );
  return button;
}

/** The breakdown shown in the "not listed" dialog. */
export function nonResultsBreakdown() {
  if (!state.nonResults.length) {
    return [el("p", { className: "quiet" }, "No breakdown is available for this sweep.")];
  }
  return [
    el(
      "p",
      { className: "quiet" },
      "Registered with the master list, but no usable reply. The in-game browser lists these anyway.",
    ),
    el(
      "dl",
      { className: "kv" },
      state.nonResults.flatMap((group) => [
        el("dt", null, String(group.count)),
        el("dd", null, nonResultReason(group)),
      ]),
    ),
  ];
}

/**
 * Write the live region, at a cadence a screen reader can survive.
 *
 * The sweep emits one event per probed endpoint, so a region that simply restated
 * "N of M done" was firing roughly two hundred announcements per sweep — which is not
 * progress reporting, it is a denial of service against the one output a blind player has
 * (docs/ux-standards.md §5.7, docs/design-review.md F23). Progress is announced at quarters
 * instead: start, three milestones, then the summary. Five utterances rather than two hundred.
 *
 * Everything else — a single check, a scope change, a failure — is a discrete event and is
 * announced when it happens. The text is only assigned when it actually differs, so a render
 * triggered by something the region does not describe stays silent.
 */
function announce(live) {
  const text = liveText();
  if (text !== live.textContent) live.textContent = text;
}

/**
 * Sweep progress, coarsened to quarters.
 *
 * Deliberately carries no live counts. A running total inside the sentence would make the string
 * differ on every probe and defeat the whole point of the milestone.
 */
const SWEEP_MILESTONES = [
  "A quarter of the servers checked.",
  "Half of the servers checked.",
  "Three quarters of the servers checked.",
];

function sweepLiveText() {
  const { probed, inspected } = state.browse;
  if (inspected <= 0) return "Getting the server list. Contacting the master server.";
  const quarter = Math.min(3, Math.floor((probed / inspected) * 4));
  return quarter === 0
    ? `Checking ${inspected} servers.`
    : SWEEP_MILESTONES[quarter - 1];
}

function liveText() {
  // A single-server check is a state change with no progress meter and, in All, no row left behind
  // when it fails. It is announced first because it is what the player just asked for.
  const single = singleCheckText();
  if (single) return single;
  if (state.scope !== "all") {
    const saved = savedEntries();
    const present = new Set(state.servers.map((row) => row.address));
    const found = saved.filter((entry) => present.has(entry.address)).length;
    const folded = state.showAbsent ? 0 : scopedAbsent().length;
    return [
      `Showing ${savedNoun()}. ${found} of ${saved.length} answered the last check.`,
      // The disclosure says this on screen; a screen reader gets it here rather than only on
      // reaching the row.
      folded > 0 && `The ${folded} not in this list are folded away.`,
    ]
      .filter(Boolean)
      .join(" ");
  }
  if (state.browse.running) return sweepLiveText();
  if (state.browse.error) {
    const failure = browseFailureText(state.browse.error);
    return [
      failure.title + ".",
      failure.remedy,
      state.staleAt && `The list from ${state.staleAt} is still on screen.`,
    ]
      .filter(Boolean)
      .join(" ");
  }
  if (state.summary) {
    return `${state.summary.getstatus_reachable} servers answered out of ${state.summary.registered} registered.`;
  }
  return "";
}

/**
 * What the last one-server check is doing or found, for the live region.
 *
 * Only the selected server is announced. A favorites batch checks several at once and reading out
 * each one would bury the sweep summary the region is otherwise for.
 */
function singleCheckText() {
  if (!state.selected) return null;
  const check = state.checks.get(state.selected);
  if (!check) return null;
  const live = state.servers.find((row) => row.address === state.selected);
  if (check.status === "checking") return `Checking ${state.selected}.`;
  if (check.status === "failed") return `The check of ${state.selected} could not run. ${check.error}`;
  if (live) return null;
  const name = check.dropped?.hostname || state.selected;
  if (check.otherGame) {
    return `${name} answered for ${GAME_LABELS[check.otherGame] ?? check.otherGame} and was removed from the list.`;
  }
  if (check.movedTo) return `${name} now publishes ${check.movedTo} as its game address.`;
  return `${name} did not answer and was removed from the list.`;
}

function signature(items) {
  const rows = items.map((item) => `${item.kind[0]}${item.address}`).join(",");
  // The checks map changes what an absent row says, so it belongs in the signature — otherwise a
  // finished check would not repaint the row that asked for it.
  const checks = [...state.checks].map(([address, check]) => `${address}${check.status}`).join(",");
  // `staleAt` draws a row of its own, so a sweep failing on top of a list has to repaint even
  // though every row it holds is unchanged.
  return `${state.scope}:${state.sort.column}:${state.sort.direction}:${state.staleAt}:${rows}:${checks}`;
}

/** Every control inside one row, in visual order. */
function rowControls(tr) {
  return [...tr.querySelectorAll("button, input, select, a[href]")];
}

/**
 * The grid's keyboard model.
 *
 * Focus lives on the **row**. Up and Down move between rows — every row, not only the selectable
 * ones, so the absent block's disclosure and its Check buttons are reachable at all. Right steps
 * into the row's controls and along them; Left steps back and, from the first control, returns to
 * the row. Escape does the same from anywhere inside a row, because a player who arrowed into a
 * star needs one key to get out again.
 *
 * Up and Down deliberately do **nothing** while focus is inside a control. That is a deliberate
 * trade recorded in docs/design-review.md: leaving a row from inside one of its buttons would
 * make the star's own arrow behaviour ambiguous, and F4's fix had to preserve it.
 */
function onRowKey(event, onSelect) {
  const current = event.target.closest("tr");
  if (!current) return;
  const insideControl = Boolean(event.target.closest("button, input, select, a[href]"));
  const controls = rowControls(current);

  if (event.key === "Escape" && insideControl) {
    event.preventDefault();
    current.focus();
    return;
  }
  if (event.key === "ArrowRight") {
    const at = controls.indexOf(event.target);
    const next = controls[at + 1] ?? (insideControl ? null : controls[0]);
    if (!next) return;
    event.preventDefault();
    next.focus();
    return;
  }
  if (event.key === "ArrowLeft") {
    if (!insideControl) return;
    event.preventDefault();
    const at = controls.indexOf(event.target);
    (controls[at - 1] ?? current).focus();
    return;
  }
  // A row contains buttons — the star, and Check on an absent row. Swallowing Enter and Space
  // here would leave them working with a mouse and dead to a keyboard.
  if (insideControl) return;

  const rows = [...event.currentTarget.children].filter((tr) => tr.tabIndex === 0 || tr.tabIndex === -1);
  const index = rows.indexOf(current);
  if (index === -1) return;

  let next = null;
  if (event.key === "ArrowDown") next = rows[Math.min(index + 1, rows.length - 1)];
  else if (event.key === "ArrowUp") next = rows[Math.max(index - 1, 0)];
  else if (event.key === "Home") [next] = rows;
  else if (event.key === "End") next = rows.at(-1);
  else if (event.key === "Enter" || event.key === " ") {
    if (!current.dataset.address) return;
    event.preventDefault();
    onSelect(current.dataset.address, { activate: true });
    return;
  } else return;

  event.preventDefault();
  next?.focus();
}

/**
 * The row context menu.
 *
 * Right-clicking a row used to open WebView2's own menu — Back, Reload, Inspect — which is the
 * loudest "this is a web page in a costume" tell a Tauri app can produce, and it sits exactly
 * where Doomseeker has offered right-click-to-bookmark for twenty years
 * (docs/ux-standards.md §7.3, docs/design-review.md F22).
 *
 * Every entry here duplicates something already reachable another way. A context menu that is the
 * only route to an action is a trap for anyone who does not think to right-click.
 */
function onRowContextMenu(event, onSelect, onCheck) {
  const tr = event.target.closest("tr[data-address], tr[data-remembered]");
  if (!tr) return;
  const address = tr.dataset.address ?? tr.dataset.remembered;
  event.preventDefault();
  event.stopPropagation();
  if (tr.dataset.address && state.selected !== address) onSelect(address);

  const live = state.servers.find((item) => item.address === address) ?? null;
  const saved = savedEntries().find((entry) => entry.address === address) ?? null;
  const subject = live ?? saved;
  const starred = favoriteAddresses().has(address);
  const queryPort = live
    ? Number(live.server.endpoint.query_port)
    : (saved?.queryPort ?? null);

  openMenu(
    [
      subject && {
        label: starred ? "Remove from favorites" : "Add to favorites",
        hint: "F",
        onSelect: () => {
          toggleFavorite(subject);
          update(() => {});
        },
      },
      queryPort !== null && {
        label: "Check this server",
        hint: "R",
        disabled: !canRecheck(address),
        onSelect: () => onCheck({ address, queryPort }),
      },
      {
        label: "Copy address",
        onSelect: () => {
          // No confirmation and no failure notice: the clipboard is unavailable in exactly the
          // contexts where saying so helps nobody, and this is a convenience, not a result.
          navigator.clipboard?.writeText(address).catch(() => {});
        },
      },
    ].filter(Boolean),
    event,
    tr,
  );
}
