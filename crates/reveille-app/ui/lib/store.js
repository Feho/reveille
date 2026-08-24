// SPDX-License-Identifier: GPL-2.0-only

// One state object and a subscribe/notify pair. Views read `state` and re-render
// on change; nothing else holds application state.

import { favourites, history, historyByAddress } from "./bookmarks.js";

const INSTALL_KEY = "reveille.install";
const FILTERS_KEY = "reveille.filters";
const ENGINES_KEY = "reveille.engines";

export const state = {
  /** The identified installation, or null while first run is unresolved. */
  install: null,
  /** Explicit engine choice for this installation. */
  engine: null,

  /** The folder accepted on a previous run, if any. */
  rememberedInstall: null,

  /** Rows as they arrive during a sweep, then the authoritative post-dedup list. */
  servers: [],
  summary: null,
  nonResults: [],

  /** Sweep progress. `running` drives the toolbar; `probed`/`inspected` the meter. */
  browse: {
    running: false,
    /** Stop was pressed; probes already in flight are still draining. */
    stopping: false,
    registered: 0,
    inspected: 0,
    probed: 0,
    answered: 0,
    nonResults: 0,
    cancelled: false,
    error: null,
    completedAt: null,
  },

  /** The selected row's address, and the join preview once it arrives. */
  selected: null,
  preview: null,
  previewProgress: null,
  previewError: null,

  /** Candidate ids the player picked for ambiguous maps, keyed by map name. */
  choices: new Map(),

  /** Install run in progress, or null. */
  installRun: null,
  joinResult: null,
  joinError: null,

  /** View state. */
  filters: { query: "", hasPeople: false, hideBlocked: false },
  sort: { column: "clients", direction: "desc" },
  /** Which population the table lists: every answering server, the starred ones, or the launched ones. */
  scope: "all",

  /**
   * What a single-server check found, keyed by address. A remembered server absent from the
   * sweep has no entry until it is checked; then it is `checking`, and afterwards either it is
   * in `servers` or the recorded reason it did not answer sits here.
   */
  checks: new Map(),
  /** Sweep completion the favourites auto-check has already run for, so it runs once. */
  autoCheckedAt: null,
};

const subscribers = new Set();

export function subscribe(handler) {
  subscribers.add(handler);
  return () => subscribers.delete(handler);
}

export function notify() {
  for (const handler of subscribers) handler();
}

/** Mutate through a callback, then notify once. */
export function update(mutate) {
  mutate(state);
  notify();
}

/* Persistence -------------------------------------------------------------- */

export function rememberInstall(root) {
  try {
    localStorage.setItem(INSTALL_KEY, root);
  } catch {
    // A launcher that cannot write a preference still works; detection reruns.
  }
}

export function recallInstall() {
  try {
    return localStorage.getItem(INSTALL_KEY);
  } catch {
    return null;
  }
}

export function rememberEngine(root, engine) {
  try {
    const choices = JSON.parse(localStorage.getItem(ENGINES_KEY) ?? "{}");
    choices[root] = engine;
    localStorage.setItem(ENGINES_KEY, JSON.stringify(choices));
  } catch {
    // The explicit in-memory choice still works for this session.
  }
}

export function recallEngine(root) {
  try {
    const engine = JSON.parse(localStorage.getItem(ENGINES_KEY) ?? "{}")[root];
    return ["original", "openmohaa", "reborn"].includes(engine) ? engine : null;
  } catch {
    return null;
  }
}

export function saveFilters() {
  try {
    localStorage.setItem(
      FILTERS_KEY,
      JSON.stringify({ ...state.filters, sort: state.sort, scope: state.scope, query: "" }),
    );
  } catch {
    // Not worth surfacing.
  }
}

export function loadFilters() {
  try {
    const saved = JSON.parse(localStorage.getItem(FILTERS_KEY) ?? "null");
    if (!saved) return;
    state.filters = { query: "", hasPeople: !!saved.hasPeople, hideBlocked: !!saved.hideBlocked };
    if (saved.sort?.column) state.sort = saved.sort;
    if (SCOPES.includes(saved.scope)) state.scope = saved.scope;
  } catch {
    // Ignore a corrupt preference rather than refusing to start.
  }
}

/* Derived ------------------------------------------------------------------ */

export const SCOPES = ["all", "favourites", "history"];

const SORTERS = {
  name: (row) => row.server.hostname.toLowerCase(),
  clients: (row) => row.server.occupancy?.clients_reported ?? -1,
  map: (row) => (row.server.current_map ?? "").toLowerCase(),
  // Every listed server answered, so a round trip always exists. The fallback sorts a server
  // that somehow lacks one to the far end rather than pretending it is instant.
  ping: (row) => row.server.status_round_trip ?? Number.MAX_SAFE_INTEGER,
  // The History scope's default. No column header owns this key, so no arrow is drawn — which is
  // right, because none of the columns is what the rows are ordered by.
  launched: (row, launches) => launches.get(row.address)?.lastLaunchedAt ?? "",
};

/** Whether a live row survives the search box and the two toolbar toggles. */
function matchesFilters(row) {
  const query = state.filters.query.trim().toLowerCase();
  if (query && !row.server.hostname.toLowerCase().includes(query)) return false;
  if (state.filters.hasPeople && (row.server.occupancy?.clients_reported ?? 0) < 1) return false;
  if (state.filters.hideBlocked && row.compatibility.state.state === "no_source") return false;
  return true;
}

/** The rows the table should show, after search, filters and sort. */
export function visibleServers() {
  return sortRows(state.servers.filter(matchesFilters));
}

function sortRows(rows) {
  const key = SORTERS[state.sort.column] ?? SORTERS.clients;
  const launches = state.sort.column === "launched" ? historyByAddress() : null;
  const direction = state.sort.direction === "asc" ? 1 : -1;
  return rows.sort((left, right) => {
    const a = key(left, launches);
    const b = key(right, launches);
    if (a === b) return left.server.hostname.localeCompare(right.server.hostname);
    return a > b ? direction : -direction;
  });
}

/**
 * What the table lists for the current scope, tagged so the view does not have to work out
 * which kind of row it is holding.
 *
 * A remembered server the current sweep did not return is **not** dropped and **not** drawn with
 * the figures it had last time. It comes back as `absent`, carrying only its address and the name
 * it was starred under, and the view says so (docs/rules.md H12). Absent entries always follow the
 * live rows: there is nothing to sort them by.
 */
export function scopedRows() {
  if (state.scope === "all") {
    return visibleServers().map((row) => ({ kind: "live", address: row.address, row }));
  }
  const remembered = state.scope === "favourites" ? favourites() : history();
  const live = new Map(state.servers.map((row) => [row.address, row]));
  const query = state.filters.query.trim().toLowerCase();

  const rows = [];
  const absent = [];
  for (const entry of remembered) {
    const row = live.get(entry.address);
    if (row) {
      // The toolbar toggles are visible and pressed, so they apply here too. Quietly ignoring
      // one in this scope would make the controls mean different things in different views.
      if (matchesFilters(row)) rows.push(row);
      continue;
    }
    // An absent entry has only a remembered name and an address to match on, and neither toggle
    // has anything to test: there are no figures.
    if (query && !entry.hostname.toLowerCase().includes(query) && !entry.address.includes(query)) {
      continue;
    }
    absent.push(entry);
  }
  return [
    ...sortRows(rows).map((row) => ({ kind: "live", address: row.address, row })),
    ...absent.map((entry) => ({ kind: "absent", address: entry.address, entry })),
  ];
}

export function selectedRow() {
  return state.servers.find((row) => row.address === state.selected) ?? null;
}
