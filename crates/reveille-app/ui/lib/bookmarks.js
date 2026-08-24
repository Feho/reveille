// SPDX-License-Identifier: GPL-2.0-only

// What Reveille remembers about servers between runs: the ones the player starred,
// and the ones it launched the game for.
//
// One honesty rule shapes the whole shape of this file (docs/rules.md H12). An entry
// stores an address, a query port, and a name — and **nothing else**. No client
// count, no map, no round trip, no compatibility state. Those are facts about a
// moment, and a remembered moment rendered in the live table would read as current.
// The data to lie with simply does not exist here.
//
// The name is stored anyway, because an absent server has to be recognisable, and
// the interface labels it as remembered rather than reported.
//
// The second rule: history records a *launch*. Reveille starts the game process and
// observes that it started. Whether the server admitted the player is decided at
// connect time by bans, capacity, a password and the ping gate, and Reveille never
// sees the answer. So `recordLaunch` is called only from a launched outcome, never
// from a refusal.
//
// Storage is localStorage, matching the install and filter preferences in store.js.
// Every access is guarded: a launcher that cannot persist a preference still works.

const KEY = "reveille.bookmarks";
const VERSION = 1;

/** Beyond this the list stops being a history and starts being a log. */
const HISTORY_LIMIT = 50;

/* Reading and writing ------------------------------------------------------- */

function read() {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!saved || saved.v !== VERSION) return { favourites: [], history: [] };
    return {
      favourites: (saved.favourites ?? []).map(entry).filter(Boolean),
      history: (saved.history ?? []).map(entry).filter(Boolean),
    };
  } catch {
    // A corrupt or unreadable store is treated as empty rather than refusing to start.
    return { favourites: [], history: [] };
  }
}

function write(store) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ v: VERSION, ...store }));
  } catch {
    // Nothing here is worth interrupting the player for; the session keeps its own copy.
  }
}

/** Accept only an entry that could still be acted on: an address and a way to re-probe it. */
function entry(saved) {
  if (!saved || typeof saved.address !== "string") return null;
  const queryPort = Number(saved.queryPort);
  if (!Number.isInteger(queryPort) || queryPort < 1 || queryPort > 65535) return null;
  return {
    address: saved.address,
    queryPort,
    hostname: typeof saved.hostname === "string" ? saved.hostname : "",
    addedAt: saved.addedAt ?? null,
    lastLaunchedAt: saved.lastLaunchedAt ?? null,
    launches: Number.isInteger(saved.launches) ? saved.launches : 0,
  };
}

/**
 * The three fields a live row contributes to an entry. `endpoint.query_port` is the
 * master's query port, not the game port, and it is what `check_server` re-probes.
 */
function identify(row) {
  return {
    address: row.address,
    queryPort: Number(row.server?.endpoint?.query_port ?? 0),
    hostname: row.server?.hostname ?? "",
  };
}

/* Favourites ---------------------------------------------------------------- */

/** Starred servers, most recently added first. */
export function favourites() {
  return read().favourites;
}

/**
 * The starred addresses as a set, read once.
 *
 * The table asks about every row on every repaint, and a per-row read would parse the store a
 * hundred-odd times a paint.
 */
export function favouriteAddresses() {
  return new Set(read().favourites.map((saved) => saved.address));
}

export function isFavourite(address) {
  return read().favourites.some((saved) => saved.address === address);
}

/**
 * Star or unstar a server. Returns the new state.
 *
 * Takes either a live row or a remembered entry, because a favourite can be unstarred from a
 * row the current sweep did not return.
 */
export function toggleFavourite(subject) {
  const identity = subject.server ? identify(subject) : entry(subject);
  if (!identity) return false;
  const store = read();
  const found = store.favourites.findIndex((saved) => saved.address === identity.address);
  if (found !== -1) {
    store.favourites.splice(found, 1);
    write(store);
    return false;
  }
  store.favourites.unshift({
    address: identity.address,
    queryPort: identity.queryPort,
    hostname: identity.hostname,
    addedAt: new Date().toISOString(),
    lastLaunchedAt: null,
    launches: 0,
  });
  write(store);
  return true;
}

/** Unstar by address, for a row that is not in the current list. */
export function forget(address) {
  const store = read();
  store.favourites = store.favourites.filter((saved) => saved.address !== address);
  write(store);
}

/* History ------------------------------------------------------------------- */

/** Servers Reveille launched the game for, most recent first. */
export function history() {
  return read().history;
}

/** Address -> entry, for the list's launched-at line and its sort. */
export function historyByAddress() {
  return new Map(read().history.map((saved) => [saved.address, saved]));
}

/**
 * Record that the game was launched against this server.
 *
 * Called only from a launched outcome. A refusal never reaches here — Reveille did
 * not start the game, so there is nothing to remember.
 */
export function recordLaunch(row) {
  const store = read();
  const previous = store.history.find((saved) => saved.address === row.address);
  store.history = store.history.filter((saved) => saved.address !== row.address);
  store.history.unshift({
    ...identify(row),
    addedAt: previous?.addedAt ?? null,
    lastLaunchedAt: new Date().toISOString(),
    launches: (previous?.launches ?? 0) + 1,
  });
  store.history = store.history.slice(0, HISTORY_LIMIT);
  write(store);
}

export function clearHistory() {
  const store = read();
  store.history = [];
  write(store);
}
