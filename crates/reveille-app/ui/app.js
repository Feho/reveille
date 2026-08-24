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
import { displayPath } from "./lib/format.js";
import {
  loadFilters,
  notify,
  recallInstall,
  selectedRow,
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
});
const join = joinView($("#detail-slot"), { onJoin: getAndJoin });
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
  $("#install-chip-engine").textContent = engineLabel(state.engine);
  servers.render();
  join.render();
}

/* First run ---------------------------------------------------------------- */

function enterServers() {
  render();
  // Nothing is known about the population yet, so start looking immediately
  // rather than making the first thing a newcomer sees an empty table.
  if (!state.servers.length && !state.browse.running) refresh();
}

function leaveServers() {
  update((next) => {
    next.install = null;
    next.selected = null;
    next.preview = null;
  });
  autoDetect(setup.render, enterServers, { skipConfirmation: false });
}

/* Browsing ----------------------------------------------------------------- */

async function refresh() {
  if (state.browse.running) return;
  update((next) => {
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
    next.autoCheckedAt = null;
  });

  try {
    const payload = await browseServers(state.install.root, state.engine);
    update((next) => {
      next.servers = payload.servers;
      next.summary = payload.summary;
      next.nonResults = payload.non_results;
      next.browse.running = false;
      next.browse.cancelled = payload.cancelled;
      next.browse.completedAt = new Date().toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
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
    const preview = await previewJoin(state.install.root, address, state.engine);
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

async function getAndJoin(row, acceptIncomplete) {
  const preview = state.preview?.address === row.address ? state.preview : null;
  const totals = preview ? shoppingTotals(preview) : { count: 0 };
  const selectedCandidateIds = [...state.choices.values()];

  update((next) => {
    next.joinError = null;
    next.joinResult = null;
    next.installRun = totals.count > 0 ? { items: new Map(), done: false } : null;
  });

  try {
    const result = await installAndLaunch(
      state.install.root,
      row.address,
      state.engine,
      selectedCandidateIds,
      acceptIncomplete,
    );
    // Only a launched outcome is remembered. A refusal means Reveille did not start the game,
    // so there is nothing that happened to record (docs/rules.md H12).
    if (result.outcome?.launch === "launched") recordLaunch(row);
    update((next) => {
      next.installRun = null;
      next.joinResult = { ...result, address: row.address };
    });
  } catch (error) {
    update((next) => {
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

/* Checking one remembered server --------------------------------------------- */

/**
 * Ask a saved server directly, without a master list.
 *
 * A favourite is often not in the sweep — the master never registered it, or it did not answer
 * in time. Until it is in the list it cannot be selected or joined, so this is what makes a
 * bookmark useful in the case that matters most.
 *
 * Takes one entry or a list of them, and probes sequentially: these are third-party servers and
 * there is no reason to burst at them.
 */
let checkToken = 0;

async function check(subject) {
  const entries = Array.isArray(subject) ? subject : [subject];
  const token = ++checkToken;

  for (const entry of entries) {
    if (token !== checkToken) return;
    update((next) => next.checks.set(entry.address, { status: "checking" }));
    let result;
    try {
      result = await checkServer(
        state.install.root,
        entry.address,
        entry.queryPort,
        state.engine,
      );
    } catch (error) {
      update((next) => next.checks.set(entry.address, { status: "failed", error: errorText(error) }));
      continue;
    }
    if (token !== checkToken) return;
    update((next) => {
      if (!result.row) {
        next.checks.set(entry.address, { status: "absent", nonResult: result.non_result });
        return;
      }
      // A server publishes its own game port, so one that moved answers at another address. The
      // row is real and joins at the address it answered on; the bookmark is left pointing where
      // the player put it rather than being silently repointed at what may be a different server.
      if (result.row.address !== entry.address) {
        next.checks.set(entry.address, { status: "absent", movedTo: result.row.address });
      } else {
        next.checks.delete(entry.address);
      }
      next.servers = [
        ...next.servers.filter((row) => row.address !== result.row.address),
        result.row,
      ];
    });
  }
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
  const typing = event.target instanceof HTMLInputElement;
  if (event.key === "/" && !typing) {
    event.preventDefault();
    servers.focusSearch();
  } else if (event.key === "Escape" && typing) {
    update((next) => (next.filters.query = ""));
    servers.focusFirstRow();
  } else if (event.key === "F5" || (event.ctrlKey && event.key === "r")) {
    event.preventDefault();
    if (!state.browse.running) refresh();
  } else if ((event.key === "f" || event.key === "F") && !typing && !event.ctrlKey) {
    const row = selectedRow();
    if (!row) return;
    event.preventDefault();
    toggleFavourite(row);
    notify();
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
