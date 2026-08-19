// SPDX-License-Identifier: GPL-2.0-only

// Boot, routing and the long-running operations. Views render from `state`;
// this module is the only place that calls commands and mutates state in
// response to them.

import { $, fill } from "./lib/dom.js";
import {
  browseServers,
  cancelBrowse,
  errorText,
  installAndLaunch,
  onBrowseProgress,
  onInstallProgress,
  onPreviewProgress,
  previewJoin,
} from "./lib/api.js";
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
  });

  try {
    const payload = await browseServers(state.install.root);
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
    next.acceptIncomplete = false;
    next.installRun = null;
    next.joinResult = null;
    next.joinError = null;
  });

  const row = selectedRow();
  // Nothing to resolve: the rotation is already satisfied, or there is none.
  if (!row || row.compatibility.state.state === "compatible") return;

  update((next) => (next.previewProgress = { index: -1, of: 0, map: "" }));
  try {
    const preview = await previewJoin(state.install.root, address);
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

async function getAndJoin(row) {
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
      selectedCandidateIds,
      state.acceptIncomplete,
    );
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
  }
});

/* Boot ---------------------------------------------------------------------- */

notify();
autoDetect(setup.render, enterServers);
