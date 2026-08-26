// SPDX-License-Identifier: GPL-2.0-only

// Boot, routing and the long-running operations. Views render from `state`;
// this module is the only place that calls commands and mutates state in
// response to them.

import { $, fill } from "./lib/dom.js";
import {
  browseServers,
  cancelBrowse,
  checkServer,
  errorText,
  installAndLaunch,
  onBrowseProgress,
  onInstallProgress,
  onPreviewProgress,
  previewJoin,
} from "./lib/api.js";
import { favourites, recordLaunch, toggleFavourite } from "./lib/bookmarks.js";
import { clockTime, displayPath } from "./lib/format.js";
import {
  GAME_LABELS,
  canRecheck,
  listIsForCurrentSession,
  loadFilters,
  notify,
  playableGames,
  recallInstall,
  rememberGame,
  selectedRow,
  session,
  state,
  subscribe,
  update,
} from "./lib/store.js";
import { autoDetect, setupView } from "./views/setup.js";
import { nonResultsBreakdown, serversView } from "./views/servers.js";
import { joinView, shoppingTotals } from "./views/join.js";

const shell = $("#shell");
const setupRoot = $("#setup-root");
const infoDialog = $("#info-dialog");

loadFilters();
state.rememberedInstall = recallInstall();

const servers = serversView({
  onRefresh: refresh,
  onCancel: stopBrowse,
  onSelect: select,
  onShowNonResults: showNonResults,
  onCheck: check,
  onGame: selectGame,
});
const join = joinView($("#detail-slot"), { onJoin: getAndJoin, onRecheck: recheck });
const setup = setupView(setupRoot, { onReady: enterServers });

$("#toolbar-slot").replaceWith(servers.toolbar);
$("#list-slot").replaceWith(servers.listPane);
$("#status-slot").replaceWith(servers.statusbar);
document.body.append(servers.live);

$("#install-chip").addEventListener("click", leaveServers);
$("#info-dialog-close").addEventListener("click", () => infoDialog.close());

subscribe(render);

function render() {
  const ready = Boolean(state.install);
  shell.classList.toggle("hidden", !ready);
  setupRoot.classList.toggle("hidden", ready);
  if (!ready) return;

  $("#install-chip-path").textContent = displayPath(state.install.root);
  $("#install-chip-engine").textContent =
    `${GAME_LABELS[state.game] ?? state.game} · ${engineLabel(state.engine)}`;
  servers.render();
  join.render();
}

/* First run ---------------------------------------------------------------- */

/**
 * Show the server list, sweeping when what is on screen is not an answer to this session.
 *
 * Setup is re-entered to change something — the folder, the engine, or which of the three games —
 * and Continue returns here with a list that was swept for the session just left. Those rows are
 * not this game's servers, and their compatibility was judged against another search path, so they
 * are dropped and swept again rather than shown under a new heading. A session that came back
 * unchanged keeps its list: re-sweeping it would cost a couple of hundred probes to arrive at the
 * same table.
 *
 * The first run has nothing on screen and no session recorded, so it sweeps for the same reason.
 */
function enterServers() {
  render();
  if (state.browse.running) return;
  if (!state.servers.length || !listIsForCurrentSession()) refresh();
}

function leaveServers() {
  update((next) => {
    next.install = null;
    next.selected = null;
    next.preview = null;
  });
  autoDetect(setup.render, enterServers, { skipConfirmation: false });
}

/**
 * Switch which of the three games this session is browsing.
 *
 * Not a filter: Allied Assault, Spearhead and Breakthrough register with the master separately and
 * read different directories on disk, so nothing already on screen is true of the new game. The
 * list is dropped and swept again rather than re-labelled.
 *
 * Every operation already in flight was started for the game being left. Each one captured its own
 * session and will still finish against it, so their *results* are stale the moment this returns —
 * an install started for Allied Assault would otherwise render its outcome into a Spearhead
 * session. Bumping all three tokens is what discards them. A join cannot be abandoned half-written,
 * so the control is refused outright while one is running rather than raced — `joining`, not
 * `installRun`, because a compatible server has nothing to download and still has a game to start.
 */
function selectGame(game) {
  if (game === state.game || state.browse.running || state.joining) return;
  if (!playableGames(state.install).includes(game)) return;
  previewToken += 1;
  checkGeneration += 1;
  joinToken += 1;
  update((next) => {
    next.game = game;
    next.checks = new Map();
    next.checkedAt = new Map();
    next.previewProgress = null;
    next.previewError = null;
    next.joinResult = null;
    next.joinError = null;
    next.joining = false;
  });
  rememberGame(state.install.root, game);
  refresh();
}

/* Browsing ----------------------------------------------------------------- */

async function refresh() {
  if (state.browse.running) return;
  // Any check still in flight is about the list this sweep is replacing.
  checkGeneration += 1;
  const swept = session();
  update((next) => {
    // Recorded before the first row arrives, because the streamed rows belong to this session
    // too, and a sweep that ends in an error still has to leave behind what it was asking.
    next.listSession = swept;
    next.browse = {
      running: true,
      stopping: false,
      registered: 0,
      inspected: 0,
      probed: 0,
      answered: 0,
      nonResults: 0,
      cancelled: false,
      error: null,
      completedAt: null,
    };
    next.servers = [];
    next.summary = null;
    next.nonResults = [];
    next.selected = null;
    next.preview = null;
    next.joinResult = null;
    // What a previous check found described a moment that has just been superseded.
    next.checks = new Map();
    next.checkedAt = new Map();
    next.autoCheckedAt = null;
  });

  try {
    const payload = await browseServers(swept);
    update((next) => {
      next.servers = payload.servers;
      next.summary = payload.summary;
      next.nonResults = payload.non_results;
      next.browse.running = false;
      next.browse.cancelled = payload.cancelled;
      next.browse.completedAt = clockTime();
    });
  } catch (error) {
    update((next) => {
      next.browse.running = false;
      next.browse.error = errorText(error);
    });
  }
}

function stopBrowse() {
  update((next) => (next.browse.stopping = true));
  cancelBrowse().catch(() => {
    // The sweep ends on its own if the message does not land.
  });
}

onBrowseProgress((progress) => {
  if (!state.browse.running) return;
  update((next) => {
    next.browse.registered = progress.registered;
    next.browse.inspected = progress.inspected;
    next.browse.probed = progress.probed;
    next.browse.answered = progress.answered;
    next.browse.nonResults = progress.non_results;
    // Streamed rows are pre-deduplication; the payload that arrives when the
    // sweep ends replaces this list with the authoritative one.
    if (progress.row) next.servers = [...next.servers, progress.row];
  });
});

/* Selecting and previewing -------------------------------------------------- */

let previewToken = 0;

async function select(address) {
  const token = ++previewToken;
  update((next) => {
    next.selected = address;
    next.preview = null;
    next.previewProgress = null;
    next.previewError = null;
    next.choices = new Map();
    next.installRun = null;
    next.joinResult = null;
    next.joinError = null;
  });

  const row = selectedRow();
  // Nothing to resolve: the rotation is already satisfied, or there is none.
  if (!row || row.compatibility.state.state === "compatible") return;

  update((next) => (next.previewProgress = { index: -1, of: 0, map: "" }));
  try {
    const preview = await previewJoin(session(), address);
    if (token !== previewToken) return;
    update((next) => {
      next.preview = preview;
      next.previewProgress = null;
    });
  } catch (error) {
    if (token !== previewToken) return;
    update((next) => {
      next.previewProgress = null;
      next.previewError = errorText(error);
    });
  }
}

onPreviewProgress((progress) => {
  if (progress.address !== state.selected) return;
  update((next) => (next.previewProgress = progress));
});

/* Getting files and joining -------------------------------------------------- */

let joinToken = 0;

async function getAndJoin(row, acceptIncomplete) {
  const token = ++joinToken;
  const preview = state.preview?.address === row.address ? state.preview : null;
  const totals = preview ? shoppingTotals(preview) : { count: 0 };
  const selectedCandidateIds = [...state.choices.values()];

  update((next) => {
    next.joinError = null;
    next.joinResult = null;
    // `installRun` covers the downloads; `joining` covers the command. A compatible server has
    // nothing to fetch, so without this the pane would look idle while the game was being started,
    // and a check finishing in that window could drop the row the outcome renders against.
    next.joining = true;
    next.installRun = totals.count > 0 ? { items: new Map(), done: false } : null;
  });

  try {
    const result = await installAndLaunch(
      session(),
      row.address,
      selectedCandidateIds,
      acceptIncomplete,
    );
    // Only a launched outcome is remembered. A refusal means Reveille did not start the game,
    // so there is nothing that happened to record (docs/rules.md H12). The launch is recorded even
    // if the session moved on — it really did happen — but its result is not rendered into a
    // session it is no longer about.
    if (result.outcome?.launch === "launched") recordLaunch(row);
    if (token !== joinToken) return;
    update((next) => {
      next.joining = false;
      next.installRun = null;
      next.joinResult = { ...result, address: row.address };
    });
  } catch (error) {
    if (token !== joinToken) return;
    update((next) => {
      next.joining = false;
      next.installRun = null;
      next.joinError = errorText(error);
    });
  }
}

onInstallProgress((progress) => {
  if (!state.installRun) return;
  update((next) => {
    const items = next.installRun.items;
    const existing = items.get(progress.map) ?? {
      map: progress.map,
      filename: progress.filename,
      received: 0,
      total: null,
    };
    items.set(progress.map, {
      ...existing,
      filename: progress.filename,
      phase: progress.phase,
      received: progress.received ?? existing.received,
      total: progress.total ?? existing.total,
      reason: progress.reason ?? existing.reason,
    });
  });
});

/* Checking one server on its own --------------------------------------------- */

/**
 * Results from before the list was replaced are discarded, and that is all this counts.
 *
 * It is a generation, not a cancellation: a sweep and a game switch both make every answer still in
 * flight an answer about a list that no longer exists. Two checks running at once do **not** cancel
 * each other — an earlier design bumped this on every call, so re-checking one server abandoned the
 * favourites batch mid-way and left the row it was probing reading "Checking…" for a request nobody
 * was waiting on.
 */
let checkGeneration = 0;

/**
 * Ask one server directly, without a master list.
 *
 * Two players want this, for opposite reasons. A favourite is often not in the sweep — the master
 * never registered it, or it did not answer in time — and until it is in the list it cannot be
 * selected or joined, so this is what makes a bookmark useful in the case that matters most. And
 * a server that *is* in the list was measured once, when the sweep ran: its map, its client count
 * and its round trip all age from that moment, and this is how one row is brought up to date
 * without spending a couple of hundred probes on the other two hundred.
 *
 * Takes one entry or a list of them, and probes sequentially: these are third-party servers and
 * there is no reason to burst at them.
 */
async function check(subject) {
  const entries = Array.isArray(subject) ? subject : [subject];
  const generation = checkGeneration;

  for (const entry of entries) {
    if (generation !== checkGeneration) return;
    // What the list holds for this address before the answer arrives — the reading this check is
    // about to replace, and the name to fall back on if it replaces it with nothing. A server
    // already dropped by an earlier check has no row left, so what that check recorded is carried
    // forward: losing it would leave the pane asking for this check unable to name what it is about.
    const before = state.servers.find((row) => row.address === entry.address) ?? null;
    const dropped = before
      ? { hostname: before.server.hostname, queryPort: entry.queryPort }
      : (state.checks.get(entry.address)?.dropped ?? null);
    update((next) => next.checks.set(entry.address, { status: "checking", dropped }));

    let result;
    try {
      result = await checkServer(session(), entry.address, entry.queryPort);
    } catch (error) {
      if (generation !== checkGeneration) return;
      update((next) =>
        next.checks.set(entry.address, { status: "failed", error: errorText(error), dropped }),
      );
      continue;
    }
    if (generation !== checkGeneration) return;
    update((next) => {
      if (!result.row) {
        // A check that ran and got no answer is evidence about now, and it outranks whatever the
        // sweep saw. A live row for this address is dropped rather than left standing with figures
        // this check has just shown are no longer current (docs/rules.md H12).
        next.checks.set(entry.address, {
          status: "absent",
          nonResult: result.non_result,
          otherGame: result.other_game,
          dropped,
        });
        next.servers = next.servers.filter((row) => row.address !== entry.address);
        next.checkedAt.delete(entry.address);
        return;
      }
      // A server publishes its own game port, so one that moved now publishes a different game
      // address. The row is real and joins at the address it published; what the player selected or
      // starred is left pointing where they put it, because a shared query port is not proof of the
      // same server. The old address keeps an entry saying where the answer came from.
      //
      // The old *row* goes, though, which is where this differs from `merge_checked_server` on the
      // Rust side: that function sees two game endpoints and cannot tell they came from one query
      // port, so it keeps both. Here the check was addressed to that query port, and nothing now
      // vouches for the game address it used to publish.
      if (result.row.address !== entry.address) {
        next.checks.set(entry.address, { status: "absent", movedTo: result.row.address, dropped });
      } else {
        next.checks.delete(entry.address);
      }
      // Both the address asked and the address that answered give way to the new row. Filtering
      // only the second would leave a server that moved listed twice, once with its old figures.
      next.servers = [
        ...next.servers.filter(
          (row) => row.address !== result.row.address && row.address !== entry.address,
        ),
        result.row,
      ];
      next.checkedAt.delete(entry.address);
      next.checkedAt.set(result.row.address, clockTime());
    });
    if (entry.address === state.selected) resettle(before, result.row);
  }
}

/**
 * The selected server, asked again on its own.
 *
 * Not offered while a sweep is running: that is already re-asking every server in the list, and
 * this row is about to be replaced by it. Nor while a join is running, where the pane belongs to
 * that command and a check coming back empty would take the row out from under it.
 */
function recheck(row) {
  if (!canRecheck(row.address)) return;
  check({ address: row.address, queryPort: Number(row.server.endpoint.query_port) });
}

/**
 * Keep the detail pane honest after a check has replaced the row beneath it.
 *
 * The catalogue lookup behind the pane is an answer about one running map, one rotation and one
 * reading of what is on disk. When the check found all of that unchanged it is still that answer,
 * and the player's source choices stand — discarding them because a client count moved would cost
 * them work for nothing. When any of it moved, or the server stopped answering, it is an answer to
 * a question no longer being asked, and it goes.
 *
 * A server that moved is **not** followed. The selection stays where the player put it and the pane
 * says where the answer came from, for the same reason a bookmark is not repointed: the two
 * addresses share a query port, which is not proof they are the same server. Following would also
 * select a row that, in Favourites or History, is not in the table at all.
 */
function resettle(before, after) {
  if (!after || after.address !== before?.address) {
    previewToken += 1;
    update((next) => {
      next.preview = null;
      next.previewProgress = null;
      next.previewError = null;
      next.choices = new Map();
    });
    return;
  }
  if (!sameJoinQuestion(before, after)) select(after.address);
}

/**
 * Whether two readings of one server pose the same join question.
 *
 * Not only the same map and rotation: `check_server` re-reads the installed maps, so a map put on
 * disk by other means between the two readings changes the answer without changing anything the
 * server published. The row's own verdict and the published checksum are what carry that, and a
 * preview kept across a change in either would price an old shopping list against a new row.
 */
function sameJoinQuestion(before, after) {
  if (!before) return false;
  return (
    before.server.current_map === after.server.current_map &&
    before.server.map_checksum === after.server.map_checksum &&
    before.compatibility.state.state === after.compatibility.state.state &&
    before.compatibility.current_map?.readiness === after.compatibility.current_map?.readiness &&
    before.server.rotation.length === after.server.rotation.length &&
    before.server.rotation.every((map, index) => map === after.server.rotation[index])
  );
}

/**
 * Check the favourites this sweep did not return, once per sweep.
 *
 * Without it, opening Favourites after a refresh shows a list of servers with no data and a row
 * of buttons to press. Once per sweep, and only for the ones actually missing, keeps it to a
 * handful of requests against a sweep that just sent a couple of hundred.
 */
function autoCheckFavourites() {
  if (state.scope !== "favourites") return;
  if (state.browse.running || !state.browse.completedAt) return;
  if (state.autoCheckedAt === state.browse.completedAt) return;
  const present = new Set(state.servers.map((row) => row.address));
  const absent = favourites().filter((entry) => !present.has(entry.address));
  state.autoCheckedAt = state.browse.completedAt;
  if (absent.length) check(absent);
}

subscribe(autoCheckFavourites);

/* Dialogs and global keys ---------------------------------------------------- */

function showNonResults() {
  fill($("#info-dialog-body"), ...nonResultsBreakdown());
  infoDialog.showModal();
}

document.addEventListener("keydown", (event) => {
  if (!state.install) return;
  // A bare letter is only a shortcut where there is nothing else for it to mean. Inside any form
  // control it is input — the search box, but also the game select, where R jumps to an option —
  // and inside an open dialog the shortcuts behind it are not reachable anyway. Alt and Meta are
  // excluded so a window or system chord is never swallowed.
  const typing =
    event.target instanceof HTMLInputElement ||
    event.target instanceof HTMLSelectElement ||
    event.target instanceof HTMLTextAreaElement ||
    event.target.isContentEditable === true ||
    Boolean(event.target.closest?.("dialog[open]"));
  const plain = !event.ctrlKey && !event.altKey && !event.metaKey;
  if (event.key === "/" && !typing) {
    event.preventDefault();
    servers.focusSearch();
  } else if (event.key === "Escape" && typing) {
    update((next) => (next.filters.query = ""));
    servers.focusFirstRow();
  } else if (event.key === "F5" || (event.ctrlKey && event.key === "r")) {
    event.preventDefault();
    if (!state.browse.running) refresh();
  } else if ((event.key === "f" || event.key === "F") && !typing && plain) {
    const row = selectedRow();
    if (!row) return;
    event.preventDefault();
    toggleFavourite(row);
    notify();
  } else if ((event.key === "r" || event.key === "R") && !typing && plain) {
    // Plain R re-asks the selected server; Ctrl+R, handled above, re-asks the whole list. The
    // modifier is the difference between one probe and a couple of hundred.
    const row = selectedRow();
    if (!row) return;
    event.preventDefault();
    recheck(row);
  }
});

/* Boot ---------------------------------------------------------------------- */

notify();
autoDetect(setup.render, enterServers);

function engineLabel(engine) {
  if (engine === "openmohaa") return "OpenMoHAA";
  if (engine === "reborn") return "Reborn";
  return "Original game";
}
