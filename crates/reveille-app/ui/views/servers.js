// SPDX-License-Identifier: GPL-2.0-only

// The server list: toolbar, table, status bar.
//
// The table is a real <table> with header semantics and arrow-key row movement,
// not a grid of buttons, so a screen reader and a keyboard both get a list of
// servers rather than a pile of controls.
//
// The Needs column carries no green and no amber. See docs/ui.md — a status
// traffic light trains people to click only green, which would push roughly a
// quarter of the live population behind a colour that reads as a warning when it
// actually means "one click of downloads".

import { el, fill } from "../lib/dom.js";
import { mapName, needsCell, nonResultReason, occupancy, shortVersion } from "../lib/format.js";
import { saveFilters, state, update, visibleServers } from "../lib/store.js";

const COLUMNS = [
  { key: "name", label: "Server", sortable: true, className: "col-name" },
  { key: "clients", label: "Clients", sortable: true, numeric: true, className: "col-clients" },
  { key: "map", label: "Map now", sortable: true, className: "col-map" },
  { key: "runs", label: "Runs", sortable: false, className: "col-runs" },
  { key: "needs", label: "Needs", sortable: false, className: "col-needs" },
];

export function serversView({ onRefresh, onCancel, onSelect, onShowNonResults }) {
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
  const hasPeople = toggle("Has people", () =>
    update((next) => {
      next.filters.hasPeople = !next.filters.hasPeople;
      saveFilters();
    }),
  );
  const hideBlocked = toggle("Hide unavailable maps", () =>
    update((next) => {
      next.filters.hideBlocked = !next.filters.hideBlocked;
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
    el(
      "label",
      { className: "field toolbar__search", for: "server-search" },
      el("span", { className: "field__icon", "aria-hidden": "true" }, "⌕"),
      search,
    ),
    hasPeople,
    hideBlocked,
    el("span", { className: "toolbar__spacer" }),
    actionSlot,
  );
  const tbody = el("tbody", { onkeydown: (event) => onRowKey(event, onSelect) });
  const table = el(
    "table",
    { className: "servers" },
    el("caption", { className: "sr-only" }, "Servers answering now"),
    el("thead", null, el("tr", null, COLUMNS.map(headerCell))),
    tbody,
  );
  const listPane = el("div", { className: "list-pane" }, table);
  const statusbar = el("div", { className: "statusbar" });
  const live = el("p", { className: "sr-only", role: "status", "aria-live": "polite" });

  let lastSignature = null;
  let lastPainted = 0;
  let pending = null;

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

  const paintRows = () => {
    lastPainted = performance.now();
    const rows = visibleServers();
    lastSignature = signature(rows);
    fill(tbody, rows.length ? rows.map((item) => row(item, onSelect)) : emptyRow());
    syncSelection();
  };

  const render = () => {
    hasPeople.setAttribute("aria-pressed", state.filters.hasPeople ? "true" : "false");
    hideBlocked.setAttribute("aria-pressed", state.filters.hideBlocked ? "true" : "false");
    if (search.value !== state.filters.query) search.value = state.filters.query;
    paintAction();
    fill(statusbar, ...statusbarContents(onShowNonResults));
    live.textContent = liveText();

    const next = signature(visibleServers());
    if (next === lastSignature) {
      syncSelection();
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

function headerCell(column) {
  const active = state.sort.column === column.key;
  const attrs = {
    scope: "col",
    className: [column.numeric ? "num" : null, column.className].filter(Boolean).join(" ") || null,
  };
  if (column.sortable && active) {
    attrs["aria-sort"] = state.sort.direction === "asc" ? "ascending" : "descending";
  }
  if (!column.sortable) return el("th", attrs, column.label);
  return el(
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
              next.sort = { column: column.key, direction: column.numeric ? "desc" : "asc" };
            }
            saveFilters();
          }),
      },
      column.label,
      active && el("span", { className: "sort-arrow" }, state.sort.direction === "asc" ? "▲" : "▼"),
    ),
  );
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

function row(item, onSelect) {
  const { clients, bots, capacity } = occupancy(item.server);
  const needs = needsCell(item.compatibility.state);
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
    el(
      "td",
      { className: "col-name" },
      el(
        "span",
        { className: "server-name", title: item.server.hostname },
        item.server.hostname || "(unnamed server)",
      ),
      el("span", { className: "server-address" }, item.address),
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
      { className: "col-runs" },
      el("span", { className: "runs-cell", title: item.server.version ?? "" }, shortVersion(item.server)),
    ),
    el(
      "td",
      { className: "col-needs" },
      needs && el("span", { className: `needs needs--${needs.kind}`, title: needs.title }, needs.text),
    ),
  );
}

function emptyRow() {
  const filtering = state.filters.query || state.filters.hasPeople || state.filters.hideBlocked;
  let body;
  if (state.browse.running) {
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
    el("td", { colspan: String(COLUMNS.length) }, el("div", { className: "placeholder" }, body)),
  );
}

function statusbarContents(onShowNonResults) {
  const { summary, browse } = state;
  if (browse.error) return [el("span", { className: "error" }, browse.error)];
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
  if (state.browse.running) {
    return `Checking servers, ${state.browse.probed} of ${state.browse.inspected} done, ${state.browse.answered} answered.`;
  }
  if (state.browse.error) return `Server check failed. ${state.browse.error}`;
  if (state.summary) {
    return `${state.summary.getstatus_reachable} servers answered out of ${state.summary.registered} registered.`;
  }
  return "";
}

function signature(rows) {
  return `${state.sort.column}:${state.sort.direction}:${rows.map((item) => item.address).join(",")}`;
}

function onRowKey(event, onSelect) {
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
