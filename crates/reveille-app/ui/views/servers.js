// SPDX-License-Identifier: GPL-2.0-only

// The server list: toolbar, table, status bar.
//
// The table is a real <table> with header semantics and arrow-key row movement,
// not a grid of buttons, so a screen reader and a keyboard both get a list of
// servers rather than a pile of controls.
//
// The list states no compatibility verdict at all — no badge, no green, no amber.
// See docs/ui.md §2.1: a status traffic light trains people to click only green,
// which would push roughly a quarter of the live population behind a colour that
// reads as a warning when it actually means "one click of downloads". What a
// server costs is said in the detail pane, where there is room to say it in the
// four canonical state names rather than a coloured word.

import { el, fill, preserveFocus } from "../lib/dom.js";
import {
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
  favouriteAddresses,
  favourites,
  history,
  historyByAddress,
  toggleFavourite,
} from "../lib/bookmarks.js";
import {
  GAME_LABELS,
  SCOPES,
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
  { key: "star", label: "Favourite", sortable: false, className: "col-star", hideLabel: true },
  { key: "name", label: "Server", sortable: true, className: "col-name" },
  { key: "clients", label: "Clients", sortable: true, numeric: true, className: "col-clients" },
  { key: "map", label: "Map now", sortable: true, className: "col-map" },
  // The gametype the server publishes, in its own spelling — see `gameType` in lib/format.js
  // for why it is not shortened to FFA/OBJ/TDM.
  { key: "mode", label: "Mode", sortable: true, className: "col-mode" },
  // Ascending first: on a distance column the useful end is the small one, which is the
  // opposite of Clients, where the busy servers are what a player is looking for.
  {
    key: "ping",
    label: "Ping",
    sortable: true,
    numeric: true,
    defaultDirection: "asc",
    className: "col-ping",
  },
  { key: "runs", label: "Runs", sortable: false, className: "col-runs" },
];

/**
 * How many columns the table is actually drawing right now.
 *
 * Not `COLUMNS.length`: the narrow-window media queries in `styles/views.css` drop Runs, then
 * Mode, then Ping, and a dropped column is gone from the table, not merely invisible. Every
 * `colspan` here has to agree with that or it runs off the end of the row — and a `colspan` that
 * overruns is not clipped. Chromium answers it by inventing the column the span asked for and
 * splitting the free width evenly between that phantom and **Server**, the only column with no
 * fixed width. That is what made the server name half its proper width in Favourites and History,
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

const SCOPE_LABELS = { all: "All", favourites: "Favourites", history: "History" };

const CAPTIONS = {
  all: "Servers answering now",
  favourites: "Starred servers, and whether this check reached them",
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

  const hasPeople = toggle("Has people", () =>
    update((next) => {
      next.filters.hasPeople = !next.filters.hasPeople;
      saveFilters();
    }),
  );
  // Both browse controls are built once and shown or hidden, never rebuilt. A
  // sweep notifies several times a second, and replacing a button between its
  // mousedown and mouseup swallows the click — which is why Stop appeared dead.
  const meterFill = el("span", { className: "meter__fill" });
  const meter = el(
    "div",
    { className: "meter", role: "progressbar", "aria-label": "Checking servers" },
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
    hasPeople,
    el("span", { className: "toolbar__spacer" }),
    actionSlot,
  );
  const tbody = el("tbody", { onkeydown: (event) => onRowKey(event, onSelect) });
  // Header cells are built once and their sort state written in place. Rebuilding them left the
  // arrow and the highlight frozen on whichever column was sorted when the view was created.
  const headers = COLUMNS.map(headerCell);
  const table = el(
    "table",
    { className: "servers" },
    el("caption", { className: "sr-only" }),
    el("thead", null, el("tr", null, headers.map((header) => header.th))),
    tbody,
  );
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
    const counts = { all: state.servers.length, favourites: favourites().length, history: history().length };
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

  const syncSelection = () => {
    for (const tr of tbody.children) {
      if (!tr.dataset.address) continue;
      tr.setAttribute("aria-selected", tr.dataset.address === state.selected ? "true" : "false");
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
    const starred = favouriteAddresses();
    for (const tr of tbody.children) {
      const address = tr.dataset.address ?? tr.dataset.remembered;
      const button = address && tr.querySelector(".star");
      if (!button) continue;
      const on = starred.has(address);
      button.setAttribute("aria-pressed", on ? "true" : "false");
      button.title = on ? "Remove from favourites" : "Add to favourites";
      button.textContent = on ? "★" : "☆";
    }
  };

  const paintRows = () => {
    lastPainted = performance.now();
    const items = scopedRows();
    lastSignature = signature(items);
    lastColumns = columnsShown(table);
    // Read the starred set once. Asking per row would parse the store a hundred-odd times a paint.
    const starred = favouriteAddresses();
    const launches = state.scope === "history" ? historyByAddress() : null;
    const build = (item) => {
      if (item.kind === "live") return row(item.row, starred, launches, onSelect);
      if (item.kind === "disclosure") return disclosureRow(item.count, lastColumns);
      return absentRow(item.entry, starred, launches, lastColumns, onCheck, onGame);
    };
    // Opening the absent block repaints the table the button that opened it lives in, and a
    // keyboard would be left on nothing. The disclosure carries a focus key so it comes back.
    preserveFocus(tbody, () =>
      fill(tbody, items.length ? items.map(build) : emptyRow(lastColumns)),
    );
    syncSelection();
  };

  const render = () => {
    hasPeople.setAttribute("aria-pressed", state.filters.hasPeople ? "true" : "false");
    if (search.value !== state.filters.query) search.value = state.filters.query;
    paintGame();
    paintScope();
    caption.textContent = CAPTIONS[state.scope];
    paintHeaders();
    paintAction();
    fill(statusbar, ...statusbarContents(onShowNonResults, onCheck));
    live.textContent = liveText();

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
    focusFirstRow: () => tbody.querySelector("tr[data-address]")?.focus(),
  };
}

/** One header cell, plus the nodes whose sort state is written on every render. */
function headerCell(column) {
  const attrs = {
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
 * Takes a live row or a remembered entry, because a favourite can be unstarred from a row this
 * sweep did not return.
 */
function starCell(subject, address, hostname, starred) {
  const on = starred.has(address);
  const name = hostname || address;
  return el(
    "td",
    { className: "col-star" },
    el(
      "button",
      {
        type: "button",
        className: "star",
        "aria-pressed": on ? "true" : "false",
        "aria-label": `Favourite ${name}`,
        title: on ? "Remove from favourites" : "Add to favourites",
        onclick: (event) => {
          event.stopPropagation();
          toggleFavourite(subject);
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
      dataset: { address: item.address },
      tabIndex: 0,
      "aria-selected": "false",
      onclick: choose,
      onfocus: choose,
    },
    starCell(item, item.address, item.server.hostname, starred),
    el(
      "td",
      { className: "col-name" },
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
      { className: "num col-clients" },
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
      { className: "col-map" },
      el(
        "span",
        { className: "map-cell" },
        item.server.current_map ? mapName(item.server.current_map) : "—",
      ),
    ),
    el(
      "td",
      { className: "col-mode" },
      el("span", { className: "mode-cell", title: mode.title }, mode.text),
    ),
    el(
      "td",
      { className: "num col-ping" },
      el("span", { className: "ping-cell", title: ping.title }, ping.text),
    ),
    el(
      "td",
      { className: "col-runs" },
      el("span", { className: "runs-cell", title: item.server.version ?? "" }, shortVersion(item.server)),
    ),
  );
}

/** The scope's own word for what it holds, singular or plural. */
const SAVED_NOUNS = {
  favourites: ["favourite", "favourites"],
  history: ["launched server", "launched servers"],
};

function savedNoun(count = 0) {
  const [one, many] = SAVED_NOUNS[state.scope] ?? ["server", "servers"];
  return count === 1 ? one : many;
}

/**
 * The head of the absent block: how many remembered entries this check did not return, and the
 * control that folds them out of the way.
 *
 * Absent entries used to sit open at the foot of Favourites and History. On a folder with more
 * than one game that is where most of them live for ever — Allied Assault, Spearhead and
 * Breakthrough register with the master separately, so a server starred under one of them is
 * never in another's list and its Check button can only ever find the same thing. Twenty rows of
 * "not in this check" under three that answered reads as a broken list.
 *
 * Shut, this is not a filter with an invisible effect (docs/ui.md §2.1, rule H15): the count is on
 * screen, in the row where the entries would have been, and one click brings them back. Nothing is
 * classified, guessed or dropped — the criterion is the same one the rows themselves state, which
 * this check demonstrably did not return.
 */
function disclosureRow(count, columns) {
  const open = state.showAbsent;
  // Naming the game earns its place only where the folder has more than one: it is why most of
  // these entries can never answer. On a single-game folder it is noise, and the game select is
  // hidden there for the same reason.
  const check =
    playableGames(state.install).length > 1
      ? `this ${GAME_LABELS[state.game] ?? state.game} check`
      : "this check";
  return el(
    "tr",
    // No `data-address`: there is nothing here to select or preview.
    { className: "row-disclosure" },
    el(
      "td",
      { colspan: String(columns) },
      el(
        "button",
        {
          type: "button",
          className: "disclosure",
          "aria-expanded": open ? "true" : "false",
          dataset: { focusKey: "absent-disclosure" },
          title:
            "These weren't in this check's list, so they were never asked. Servers saved under another game always end up here.",
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
 * says "not in this check" across the columns where every figure would have been, so there is
 * nothing here a reader could take for a current measurement. The italic `--remembered` name
 * carries the rest.
 *
 * The wording is "not in this check", not "offline". The sweep asks the master for a list and
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
    { className: "row-absent", dataset: { remembered: entry.address } },
    starCell(entry, entry.address, entry.hostname, starred),
    el(
      "td",
      { className: "col-name" },
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
      { colspan: String(columns - 2) },
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
        "This server was not in the list the master server returned for this check, so it was never asked. Check it on its own to find out.",
    },
    "not in this check",
  );
}

function emptyRow(columns) {
  const filtering = state.filters.query || state.filters.hasPeople;
  let body;
  // A scope with nothing saved and a scope whose entries are all filtered out are different
  // problems, and only one of them is fixed by clearing the search box.
  if (state.scope === "favourites" && favourites().length === 0) {
    body = [el("h3", null, "No favourites yet"), el("p", null, "Star a server to keep it here.")];
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
      el("h3", null, "Checking servers"),
      el("p", null, "Rows appear as each server answers."),
    ];
  } else if (state.servers.length) {
    body = [
      el("h3", null, "Nothing matches"),
      el(
        "p",
        null,
        filtering
          ? "No server matches the current search and filters."
          : "The list is empty for this view.",
      ),
    ];
  } else {
    body = [
      el("h3", null, "No servers yet"),
      el(
        "p",
        null,
        "Nothing has been checked yet.",
      ),
    ];
  }
  return el(
    "tr",
    null,
    el("td", { colspan: String(columns) }, el("div", { className: "placeholder" }, body)),
  );
}

function statusbarContents(onShowNonResults, onCheck) {
  const { summary, browse } = state;
  if (browse.error) return [el("span", { className: "error" }, browse.error)];
  if (state.scope !== "all") return scopedStatusbar(onCheck);
  if (!summary && !browse.running) return [el("span", null, "Not checked yet")];

  const answered = summary ? summary.getstatus_reachable : browse.answered;
  const registered = summary ? summary.registered : browse.registered;
  const skipped = summary ? summary.non_results : browse.nonResults;
  return [
    el("span", null, el("strong", null, String(answered)), ` of ${registered} answered`),
    summary &&
      el(
        "span",
        { title: "Occupied slots reported by every server. Not verified as people." },
        el("strong", null, String(summary.clients_reported)),
        " clients reported",
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
        ` of ${saved.length} ${savedNoun(saved.length)} in this check`,
      ),
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
      folded > 0 && `The ${folded} not in this check are folded away.`,
    ]
      .filter(Boolean)
      .join(" ");
  }
  if (state.browse.running) {
    return `Checking servers, ${state.browse.probed} of ${state.browse.inspected} done, ${state.browse.answered} answered.`;
  }
  if (state.browse.error) return `Server check failed. ${state.browse.error}`;
  if (state.summary) {
    return `${state.summary.getstatus_reachable} servers answered out of ${state.summary.registered} registered.`;
  }
  return "";
}

/**
 * What the last one-server check is doing or found, for the live region.
 *
 * Only the selected server is announced. A favourites batch checks several at once and reading out
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
  return `${state.scope}:${state.sort.column}:${state.sort.direction}:${rows}:${checks}`;
}

function onRowKey(event, onSelect) {
  // A row contains buttons — the star, and Check on an absent row. Swallowing Enter and Space
  // here would leave them working with a mouse and dead to a keyboard.
  if (event.target.closest("button")) return;
  const current = event.target.closest("tr[data-address]");
  if (!current) return;
  const rows = [...event.currentTarget.querySelectorAll("tr[data-address]")];
  const index = rows.indexOf(current);
  if (index === -1) return;

  let next = null;
  if (event.key === "ArrowDown") next = rows[Math.min(index + 1, rows.length - 1)];
  else if (event.key === "ArrowUp") next = rows[Math.max(index - 1, 0)];
  else if (event.key === "Home") [next] = rows;
  else if (event.key === "End") next = rows.at(-1);
  else if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    onSelect(current.dataset.address, { activate: true });
    return;
  } else return;

  event.preventDefault();
  next?.focus();
}
