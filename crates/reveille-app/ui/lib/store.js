// SPDX-License-Identifier: GPL-2.0-only

// One state object and a subscribe/notify pair. Views read `state` and re-render
// on change; nothing else holds application state.

import { favorites, history, historyByAddress } from "./bookmarks.js";

const INSTALL_KEY = "reveille.install";
const FILTERS_KEY = "reveille.filters";
const ENGINES_KEY = "reveille.engines";
const GAMES_KEY = "reveille.games";

/** The three games, as the Rust side spells them. Also how `Installation.products` spells them. */
export const GAMES = ["allied_assault", "spearhead", "breakthrough"];

export const GAME_LABELS = {
  allied_assault: "Allied Assault",
  spearhead: "Spearhead",
  breakthrough: "Breakthrough",
};

export const state = {
  /** A newer signed Reveille release retained by the Rust updater, when one was found. */
  selfUpdate: { offer: null, running: false, stopping: false, progress: null, error: null },

  /** The identified installation, or null while first run is unresolved. */
  install: null,
  /** Explicit engine choice for this installation. */
  engine: null,
  /**
   * Which of the three games this session is browsing and joining.
   *
   * It is not a filter over one list: each game has its own master registration, its own servers,
   * and its own search path on disk, so changing it starts a different sweep.
   */
  game: "allied_assault",

  /** The folder accepted on a previous run, if any. */
  rememberedInstall: null,

  /** Rows as they arrive during a sweep, then the authoritative post-dedup list. */
  servers: [],
  summary: null,
  nonResults: [],

  /**
   * The session the rows on screen were swept for, or null before the first sweep.
   *
   * The list is not a view of "the servers": it is the answer to one particular question — this
   * folder, this engine, this game. Nothing on screen says which, so a list left over from a
   * session that has since changed would read as a current answer to the new question. Keeping
   * what it was swept for is what lets a returning session tell the difference.
   */
  listSession: null,

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

  /** Install run in progress, or null. Downloads only — see `joining` for the whole command. */
  installRun: null,
  /**
   * Whether `install_and_launch` is running.
   *
   * Wider than `installRun`, which is null when a compatible server has nothing to fetch. The join
   * command owns the detail pane for its whole length, downloads or not, so this is what the pane
   * and the one-server check read.
   */
  joining: false,
  joinResult: null,
  joinError: null,

  /**
   * View state.
   *
   * `notEmpty` was called `hasPeople` until 27 Aug 2026. A client count is occupied slots and
   * cannot distinguish a person from a bot or a parked connection, so the old name asserted in the
   * toolbar exactly what the status bar four inches away says is not verified (rule H1,
   * docs/design-review.md F7). The identifier moved with the label, because a name that reads as a
   * claim is how the claim gets back onto the screen.
   *
   * `maxPing` gates on the one round trip this sweep measured, not on the in-game ping — see
   * `roundTrip` in lib/format.js. Null means no gate.
   */
  filters: { query: "", notEmpty: false, maxPing: null },
  sort: { column: "clients", direction: "desc" },
  /** Which population the table lists: every answering server, the starred ones, or the launched ones. */
  scope: "all",
  /**
   * Whether a saved scope's absent block is open.
   *
   * Shut by default. What it hides is stated on the disclosure that hides it, so a player can see
   * that entries are folded away and how many (H15) — this is not a filter with an invisible
   * effect, which is what got the old "Hide unavailable maps" toggle removed (docs/ui.md §2.1).
   */
  showAbsent: false,

  /**
   * What a single-server check found, keyed by address. A remembered server absent from the
   * sweep has no entry until it is checked; then it is `checking`, and afterwards either it is
   * in `servers` or the recorded reason it did not answer sits here.
   */
  checks: new Map(),
  /**
   * When a single-server check last measured each address, as a clock time.
   *
   * Only the servers a check re-asked on their own are in here, one entry per re-check rather
   * than one per row. A row still carrying the sweep's own figures has no entry, and the pane
   * words it as the check it came from rather than as a measurement of that one server: probes
   * stream in across a whole sweep, so its finish time is not when any particular row answered.
   */
  checkedAt: new Map(),
  /** Sweep completion the favorites auto-check has already run for, so it runs once. */
  autoCheckedAt: null,

  /**
   * When the rows on screen were measured, once a sweep has failed on top of them.
   *
   * A sweep that cannot reach the master used to blank the table and leave the centre of the
   * window reading "Nothing has been checked yet" underneath an error in the corner — the two
   * contradicting each other, with no next action in either (docs/design-review.md F6). The rows
   * from the last sweep that did work are kept instead, and this is the clock time that says what
   * they are: a past reading, not a current one (docs/ux-standards.md §4.5).
   *
   * Null whenever the list on screen is this session's own answer.
   */
  staleAt: null,
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

/**
 * Remember the game per install folder, like the engine.
 *
 * Per folder rather than globally: a second installation may not have the same expansions, and a
 * remembered game its data directories cannot serve would be a session that fails on its first
 * command.
 */
export function rememberGame(root, game) {
  try {
    const choices = JSON.parse(localStorage.getItem(GAMES_KEY) ?? "{}");
    choices[root] = game;
    localStorage.setItem(GAMES_KEY, JSON.stringify(choices));
  } catch {
    // The explicit in-memory choice still works for this session.
  }
}

export function recallGame(root) {
  try {
    const game = JSON.parse(localStorage.getItem(GAMES_KEY) ?? "{}")[root];
    return GAMES.includes(game) ? game : null;
  } catch {
    return null;
  }
}

/**
 * The games an install can actually run, which is not the same as the products detected in it:
 * an expansion needs the base game underneath it, and the Rust side decides that (rules H13/H14).
 */
export function playableGames(install) {
  return install?.playable ?? [];
}

/**
 * The game a session should open on: the remembered one when this install can still run it,
 * otherwise the first game it can.
 */
export function defaultGame(install) {
  const games = playableGames(install);
  const remembered = install ? recallGame(install.root) : null;
  if (remembered && games.includes(remembered)) return remembered;
  return games[0] ?? "allied_assault";
}

/** The three facts every server-facing command needs. */
export function session() {
  return { path: state.install.root, engine: state.engine, game: state.game };
}

/**
 * Whether the rows on screen were swept for the session in force now.
 *
 * All three facts count. The game decides which master registration was asked and which servers
 * exist at all; the folder and the engine decide the search path every row's compatibility was
 * judged against. A change to any of them makes the list an answer to a question no longer being
 * asked.
 */
export function listIsForCurrentSession() {
  const swept = state.listSession;
  if (!swept || !state.install) return false;
  const now = session();
  return swept.path === now.path && swept.engine === now.engine && swept.game === now.game;
}

export function saveFilters() {
  try {
    localStorage.setItem(
      FILTERS_KEY,
      JSON.stringify({
        notEmpty: state.filters.notEmpty,
        maxPing: state.filters.maxPing,
        sort: state.sort,
        scope: state.scope,
        showAbsent: state.showAbsent,
        query: "",
      }),
    );
  } catch {
    // Not worth surfacing.
  }
}

export function loadFilters() {
  try {
    const saved = JSON.parse(localStorage.getItem(FILTERS_KEY) ?? "null");
    if (!saved) return;
    // `hasPeople` is the pre-rename key. Read once so an existing player's toggle survives the
    // rename; nothing writes it any more.
    state.filters = {
      query: "",
      notEmpty: !!(saved.notEmpty ?? saved.hasPeople),
      maxPing: PING_LIMITS.includes(saved.maxPing) ? saved.maxPing : null,
    };
    if (saved.sort?.column) state.sort = saved.sort;
    // `favourites` is the pre-rename scope value, mapped so a player who left the app on that
    // tab comes back to it rather than to All.
    const scope = saved.scope === "favourites" ? "favorites" : saved.scope;
    if (SCOPES.includes(scope)) state.scope = scope;
    state.showAbsent = !!saved.showAbsent;
  } catch {
    // Ignore a corrupt preference rather than refusing to start.
  }
}

/* Derived ------------------------------------------------------------------ */

export const SCOPES = ["all", "favorites", "history"];

const SORTERS = {
  name: (row) => row.server.hostname.toLowerCase(),
  clients: (row) => row.server.occupancy?.clients_reported ?? -1,
  map: (row) => (row.server.current_map ?? "").toLowerCase(),
  // A server that publishes no gametype sorts to the end of an ascending sort rather than to the
  // top with the empty string, where a run of blanks would hide the modes the player is scanning.
  mode: (row) => (row.server.game_type ?? "￿").toLowerCase(),
  // Every listed server answered, so a round trip always exists. The fallback sorts a server
  // that somehow lacks one to the far end rather than pretending it is instant.
  ping: (row) => row.server.status_round_trip ?? Number.MAX_SAFE_INTEGER,
  // The History scope's default. No column header owns this key, so no arrow is drawn — which is
  // right, because none of the columns is what the rows are ordered by.
  launched: (row, launches) => launches.get(row.address)?.lastLaunchedAt ?? "",
};

/**
 * The round-trip ceilings the toolbar offers. Null is the default: no gate.
 *
 * A sort is not a filter. Sorting by players surfaces full servers on the other side of the
 * world; sorting by ping surfaces empty ones next door. Shipping the sort without the filter is a
 * documented failure across several modern browsers, and Doomseeker has had this since the 2000s
 * (docs/ux-standards.md §7, docs/design-review.md F15).
 */
export const PING_LIMITS = [null, 80, 150, 250];

/**
 * Whether a live row survives the search box and the toolbar filters.
 *
 * The query matches the **address** as well as the name. It matched only the name here while
 * `partitionScope` below matched both, so pasting an IP into All said "Nothing matches" with the
 * server on screen, and the same paste in Favorites found it (docs/design-review.md F13).
 */
function matchesFilters(row) {
  const query = state.filters.query.trim().toLowerCase();
  if (query) {
    const name = row.server.hostname.toLowerCase();
    if (!name.includes(query) && !row.address.includes(query)) return false;
  }
  if (state.filters.notEmpty && (row.server.occupancy?.clients_reported ?? 0) < 1) return false;
  const limit = state.filters.maxPing;
  // A server that published no round trip is not gated by a ceiling it cannot be measured
  // against: hiding it would be a claim about a figure that does not exist.
  const trip = row.server.status_round_trip;
  if (limit !== null && trip !== null && trip !== undefined && Number(trip) > limit) return false;
  return true;
}

/** Whether any filter is narrowing the list right now. */
export function filtering() {
  return Boolean(
    state.filters.query.trim() || state.filters.notEmpty || state.filters.maxPing !== null,
  );
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

/** The entries a saved scope draws from: the starred ones, or the launched ones. */
export function savedEntries() {
  return state.scope === "favorites" ? favorites() : history();
}

/**
 * Split a saved scope into what this check returned and what it did not, after the search box.
 *
 * One pass, read by everything that counts either half — the table, the status bar and the live
 * region — so the three cannot disagree about how many entries are in each.
 */
function partitionScope() {
  const remembered = savedEntries();
  const live = new Map(state.servers.map((row) => [row.address, row]));
  const query = state.filters.query.trim().toLowerCase();

  const rows = [];
  const absent = [];
  for (const entry of remembered) {
    const row = live.get(entry.address);
    if (row) {
      // The toolbar toggle is visible and pressed, so it applies here too. Quietly ignoring it
      // in this scope would make the control mean different things in different views.
      if (matchesFilters(row)) rows.push(row);
      continue;
    }
    // An absent entry has only a remembered name and an address to match on, and the toggle has
    // nothing to test: there are no figures.
    if (query && !entry.hostname.toLowerCase().includes(query) && !entry.address.includes(query)) {
      continue;
    }
    absent.push(entry);
  }
  return { rows, absent };
}

/** The remembered entries in this scope that the current check did not return. */
export function scopedAbsent() {
  if (state.scope === "all") return [];
  return partitionScope().absent;
}

/**
 * What the table lists for the current scope, tagged so the view does not have to work out
 * which kind of row it is holding.
 *
 * A remembered server the current sweep did not return is **not** dropped and **not** drawn with
 * the figures it had last time. It comes back as `absent`, carrying only its address and the name
 * it was starred under, and the view says so (docs/rules.md H12). Absent entries always follow the
 * live rows: there is nothing to sort them by.
 *
 * They are also **collapsed behind a disclosure that states how many there are** (H15). Each of
 * the three games registers with the master separately, so a server starred while browsing another
 * one can never appear in this check and would otherwise sit in the list for ever, unanswerable —
 * often outnumbering the rows that did answer. The `disclosure` item is emitted whenever there is
 * anything behind it, open or shut, so the count is on screen either way: rows may be folded away,
 * never silently dropped.
 */
export function scopedRows() {
  if (state.scope === "all") {
    return visibleServers().map((row) => ({ kind: "live", address: row.address, row }));
  }
  const { rows, absent } = partitionScope();
  const listed = sortRows(rows).map((row) => ({ kind: "live", address: row.address, row }));
  if (!absent.length) return listed;
  return [
    ...listed,
    // The count and the open state ride in `address` because that is what the view's row
    // signature hashes: without them, opening the block would not repaint the table.
    { kind: "disclosure", address: `${absent.length}:${state.showAbsent}`, count: absent.length },
    ...(state.showAbsent
      ? absent.map((entry) => ({ kind: "absent", address: entry.address, entry }))
      : []),
  ];
}

/**
 * Whether one server may be asked again right now.
 *
 * Not while a sweep is running — that is already re-asking every server in the list, this one
 * included. Not while a join is running — the pane belongs to that command, and a check that came
 * back empty would drop the row its progress is drawn against. And not while this address already
 * has a request in flight.
 *
 * One question, read by both the control and the handler behind it, so the two cannot drift into
 * disagreeing about when the control works.
 */
export function canRecheck(address) {
  if (state.browse.running || state.joining) return false;
  return state.checks.get(address)?.status !== "checking";
}

export function selectedRow() {
  return state.servers.find((row) => row.address === state.selected) ?? null;
}
