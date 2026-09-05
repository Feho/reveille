// SPDX-License-Identifier: GPL-2.0-only

import { el, fill } from "../lib/dom.js";
import {
  cancelOpenMohaaInstall, cancelRebornInstall, detectInstall, engineOverview, errorText,
  installOpenMohaa, installReborn, onOpenMohaaInstallProgress, onRebornInstallProgress,
  openMohaaStatus, pickInstallFolder, selectEngine,
} from "../lib/api.js";
import { bytes, displayPath } from "../lib/format.js";
import {
  GAME_LABELS, defaultGame, notify, playableGames, recallEngine, rememberEngine, rememberGame,
  rememberInstall, state,
} from "../lib/store.js";

const PRODUCT_NAMES = { allied_assault: "Allied Assault", spearhead: "Spearhead", breakthrough: "Breakthrough" };
const DESCRIPTIONS = {
  openmohaa: "Modern rebuilt game program.",
  reborn: "Classic game program with community fixes.",
  original: "Existing game program without community-engine updates.",
};
const LABELS = { openmohaa: "OpenMoHAA", reborn: "Reborn", original: "Original game" };

const view = {
  candidate: null, eyebrow: "First run", message: "Checking the usual locations.", busy: true,
  error: null, manualPath: "", overview: null, selected: null, channel: "stable", game: null,
  openStatus: null, openError: null, installing: null, stopping: false, progress: null, result: null,
};
let loadToken = 0;

export function setupView(root, { onReady, onUpdate }) {
  const render = () => fill(root, card(render, onReady, onUpdate));
  const renderUpdateOffer = () => {
    const button = root.querySelector("[data-self-update-offer]");
    if (button) button.classList.toggle("hidden", !state.selfUpdate.offer);
  };
  void onOpenMohaaInstallProgress((progress) => progressFor("openmohaa", progress, render));
  void onRebornInstallProgress((progress) => progressFor("reborn", progress, render));
  render();
  return { render, renderUpdateOffer };
}

function progressFor(engine, progress, render) {
  if (view.installing !== engine) return;
  view.progress = progress;
  render();
}

function card(render, onReady, onUpdate) {
  const available = selectedAvailable();
  return el("div", { className: "setup" }, el("div", { className: "setup__card" },
    el("div", { className: "setup__brand" }, el("span", { className: "wordmark" }, "Reveille"), el("span", { className: "label" }, view.eyebrow)),
    el("h1", { className: "setup__title" }, view.busy ? "Looking for Allied Assault" : view.candidate ? "How do you want to run the game?" : "Show Reveille your game"),
    el("p", { className: "setup__lede" }, view.message),
    view.candidate && foundBlock(view.candidate),
    view.candidate && gameChoice(view.candidate, render),
    view.candidate && engineChoices(view.candidate, render),
    !view.busy && !view.candidate && manualBlock(render),
    view.candidate && el("div", { className: "actions__row" },
      el("button", { className: "btn btn--primary btn--block", disabled: view.busy || Boolean(view.installing) || !available, onclick: () => void accept(view.candidate, onReady, render) }, available ? "Continue to servers" : "Choose an available engine"),
      el("button", { className: "btn btn--ghost", disabled: view.busy || Boolean(view.installing), onclick: () => { resetCandidate(); view.message = "Pick your game folder."; render(); } }, "Choose another folder")),
    view.error && el("p", { className: "error", role: "alert" }, view.error),
    el("div", { className: "setup__foot" },
      el("button", { type: "button", className: `btn btn--sm btn--primary ${state.selfUpdate.offer ? "" : "hidden"}`, "data-self-update-offer": true, disabled: Boolean(view.installing), onclick: onUpdate }, "Update Reveille")),
  ));
}

function foundBlock(install) {
  const products = (install.products ?? []).map((product) => PRODUCT_NAMES[product] ?? product);
  return el("div", { className: "setup__found" },
    el("p", { className: "setup__path" }, displayPath(install.root)),
    el("p", { className: "quiet" }, products.length ? products.join(" · ") : "No recognised game data"));
}

/**
 * Which of the three games this session opens on.
 *
 * Asked here, and only when the folder can run more than one, because Continue starts a search
 * immediately: a player told to choose afterwards would find the toolbar's switch disabled while
 * that first search ran. One question with an obvious default costs less than an unwanted sweep.
 */
function gameChoice(install, render) {
  const games = playableGames(install);
  if (games.length < 2) return false;
  return el("fieldset", { className: "game-choice", disabled: Boolean(view.installing) },
    el("legend", { className: "label" }, "Which game do you want to play?"),
    games.map((game) => {
      const id = `game-${game}`;
      return el("span", { className: "game-choice__option" },
        el("input", { id, type: "radio", name: "game-choice", value: game, checked: view.game === game,
          onchange: () => { view.game = game; render(); } }),
        el("label", { for: id }, GAME_LABELS[game] ?? game));
    }));
}

function engineChoices(install, render) {
  if (!view.overview) return el("div", { className: "meter meter--indeterminate", role: "status" }, el("span", { className: "meter__fill" }));
  return el("fieldset", { className: "engine-cards", disabled: Boolean(view.installing) },
    el("legend", { className: "sr-only" }, "Choose how to run the game"),
    engineCard("openmohaa", install, render), engineCard("reborn", install, render), engineCard("original", install, render),
    view.overview.selection_error && !view.selected && el("p", { className: "note" }, "Both community engines are installed. Choose the one you want to use."));
}

function engineCard(engine, install, render) {
  const selected = view.selected === engine;
  const installed = isInstalled(engine);
  const active = view.overview.resolved === engine;
  const id = `engine-${engine}`;
  return el("div", { className: `engine-card ${selected ? "engine-card--selected" : ""}` },
    el("input", { id, type: "radio", name: "engine-choice", value: engine, checked: selected, onchange: () => void chooseEngine(engine, install, render) }),
    el("span", { className: "engine-card__body" },
      el("label", { for: id, className: "engine-card__label" },
        el("span", { className: "row-between" }, el("strong", null, LABELS[engine]), statusChip(installed, active)),
        el("span", { className: "quiet" }, DESCRIPTIONS[engine])),
      selected && engineDetails(engine, install, render)));
}

function statusChip(installed, active) {
  if (active) return el("span", { className: "chip chip--ok" }, "Selected · active");
  if (installed) return el("span", { className: "chip chip--brass" }, "Installed");
  return el("span", { className: "chip chip--plain" }, "Not installed");
}

function engineDetails(engine, install, render) {
  if (engine === "original") return isInstalled(engine) ? false : el("span", { className: "note note--bad" }, "No original game program was found.");
  return engine === "reborn" ? rebornDetails(install, render) : openDetails(install, render);
}

function rebornDetails(install, render) {
  const info = view.overview.reborn;
  const build = view.overview.inventory.reborn_build;
  return el("span", { className: "engine-card__details" },
    el("span", { className: "kv-line" }, el("span", null, "Version"), el("strong", null, info.version)),
    el("span", { className: "kv-line" }, el("span", null, "Download"), el("strong", { title: info.filename }, bytes(info.size))),
    el("span", { className: "note", title: info.sha256 }, "The download must match the exact package Reveille was built to install."),
    build?.state === "known_other" && el("span", { className: "quiet" }, `${build.version} is installed. Reveille will not call it current.`),
    build?.state === "unknown" && el("span", { className: "quiet" }, "Reborn files are present, but this version is unknown."),
    !info.supported && el("span", { className: "note note--bad" }, "This legacy Reborn package supports Windows only."),
    view.installing === "reborn" ? installProgress(render) : !isInstalled("reborn") && el("button", { type: "button", className: "btn btn--primary", disabled: !info.supported, onclick: (event) => { event.preventDefault(); void runRebornInstall(install, render); } }, "Install Reborn"),
    view.result?.engine === "reborn" && el("span", { className: "note note--brass" }, "Reborn is installed and active."));
}

function openDetails(install, render) {
  const status = view.openStatus;
  return el("span", { className: "engine-card__details" },
    el("label", { className: "engine-choice" }, el("span", { className: "engine-choice__label" }, "Version"),
      el("select", { value: view.channel, onchange: (event) => { event.preventDefault(); view.channel = event.target.value; void loadOpenStatus(install, render); } },
        el("option", { value: "stable", selected: view.channel === "stable" }, "Stable"), el("option", { value: "preview", selected: view.channel === "preview" }, "Preview — less tested"))),
    view.openError && el("span", { className: "note note--bad", title: view.openError }, "Release details could not be checked right now."),
    status?.availability === "unsupported" && el("span", { className: "note note--bad" }, "OpenMoHAA is unavailable for this computer."),
    status?.availability === "available" && el("span", { className: "kv-line" }, el("span", null, "Version"), el("strong", null, status.package.prerelease ? `${status.package.version} — preview` : status.package.version)),
    status?.availability === "available" && el("span", { className: "kv-line" }, el("span", null, "Download"), el("strong", { title: status.package.asset_name }, bytes(status.package.size))),
    status?.availability === "available" && openBuildNote(status.installed_build),
    status?.availability === "available" && el("span", { className: "note", title: status.package.digest }, "Reveille checks that the download arrived intact before installing it."),
    view.installing === "openmohaa" ? installProgress(render) : status?.availability === "available" && !isInstalled("openmohaa") && el("button", { type: "button", className: "btn btn--primary", onclick: (event) => { event.preventDefault(); void runOpenInstall(install, render); } }, "Install OpenMoHAA"));
}

function openBuildNote(build) {
  if (build?.state === "current") return el("span", { className: "quiet" }, "This exact version is installed.");
  if (build?.state === "known_other") return el("span", { className: "quiet" }, `${build.version} is installed.`);
  if (build?.state === "unknown") return el("span", { className: "quiet" }, "OpenMoHAA is installed, but this version is unknown.");
  return false;
}

function installProgress(render) {
  const received = view.progress?.received ?? 0;
  const total = view.progress?.total ?? null;
  const percent = total ? Math.min(100, (received / total) * 100) : null;
  return el("span", { className: "stack--tight" },
    el("span", { className: `meter ${percent === null ? "meter--indeterminate" : ""}` }, el("span", { className: "meter__fill", style: percent === null ? null : `width: ${percent}%` })),
    el("span", { className: "quiet data" }, total ? `${bytes(received)} of ${bytes(total)}` : "Preparing download"),
    el("button", { type: "button", className: "btn btn--ghost", disabled: view.stopping, onclick: (event) => { event.preventDefault(); void stopInstall(render); } }, view.stopping ? "Stopping…" : "Stop download"));
}

/** Take an identified folder as the candidate, keeping the game choice valid for it. */
function adoptCandidate(install) {
  view.candidate = install;
  if (!playableGames(install).includes(view.game)) view.game = defaultGame(install);
}

function isInstalled(engine) { return Boolean(view.overview?.inventory?.[`${engine}_installed`]); }
function selectedAvailable() {
  if (!view.selected || !view.overview) return false;
  if (view.selected === "reborn" && !view.overview.reborn.supported) return false;
  return isInstalled(view.selected);
}

async function chooseEngine(engine, install, render) {
  view.selected = engine; view.error = null; view.result = null; render();
  if (engine === "openmohaa" && !view.openStatus) await loadOpenStatus(install, render);
}

async function loadOverview(install, render) {
  const token = ++loadToken;
  try {
    const overview = await engineOverview(install.root, recallEngine(install.root));
    if (token !== loadToken) return;
    view.overview = overview; view.selected = overview.resolved;
    if (view.selected === "openmohaa" || overview.inventory.openmohaa_installed) void loadOpenStatus(install, render);
  } catch (error) { if (token === loadToken) view.error = errorText(error); }
  render();
}

async function loadOpenStatus(install, render) {
  view.openError = null; render();
  try { view.openStatus = await openMohaaStatus(install.root, view.channel); }
  catch (error) { view.openStatus = null; view.openError = error?.detail ?? errorText(error); }
  render();
}

async function runOpenInstall(install, render) {
  if (!view.openStatus?.package) return;
  beginInstall("openmohaa", render);
  try { await installOpenMohaa(install.root, view.openStatus.package.offer_id); await reloadAfterInstall(install, "openmohaa", render); }
  catch (error) { view.error = error?.detail ?? errorText(error); finishInstall(render); }
}

async function runRebornInstall(install, render) {
  beginInstall("reborn", render);
  try { await installReborn(install.root); await reloadAfterInstall(install, "reborn", render); }
  catch (error) { view.error = errorText(error); finishInstall(render); }
}

function beginInstall(engine, render) { view.installing = engine; view.stopping = false; view.progress = null; view.error = null; view.result = null; render(); }
async function reloadAfterInstall(install, engine, render) {
  view.result = { engine }; view.installing = null; view.progress = null;
  rememberEngine(install.root, engine);
  view.overview = await engineOverview(install.root, engine); view.selected = engine;
  adoptCandidate((await detectInstall(install.root)) ?? install); render();
}
function finishInstall(render) { view.installing = null; view.stopping = false; view.progress = null; render(); }
async function stopInstall(render) {
  view.stopping = true; render();
  try { await (view.installing === "reborn" ? cancelRebornInstall() : cancelOpenMohaaInstall()); }
  catch (error) { view.error = errorText(error); view.stopping = false; render(); }
}

/**
 * What Reveille needs on disk, said before the player spends any effort looking for it.
 *
 * Setup offers to install a game *program* — OpenMoHAA or Reborn. It cannot install the game
 * *data*, which `install::identify` requires, and nothing on screen used to draw that line. A
 * player pointing at an empty folder got "No Medal of Honor installation there" and no way to
 * work out why, which is the hardest kind of wall: no error, no next action, at second zero
 * (docs/friction.md F1, docs/design-review.md F2).
 *
 * Doomseeker settled the same boundary for the same kind of player years ago — it fetches the
 * optional content and states the base game as a precondition up front, rather than as a failure
 * halfway through (docs/ux-standards.md §7, prior art).
 */
function prerequisite() {
  return el(
    "div",
    { className: "setup__needs" },
    el(
      "p",
      null,
      "Reveille needs the game files from your own copy of Medal of Honor: a disc install, GOG or the EA App.",
    ),
    el(
      "p",
      { className: "quiet" },
      "Reveille can install the game program for you, but not the game itself. Look for the folder that contains a ",
      el("span", { className: "data" }, "main"),
      " folder.",
    ),
  );
}

function manualBlock(render) {
  return el("div", { className: "stack" }, prerequisite(), el("div", { className: "setup__row" },
    el("label", { className: "field", for: "install-path" }, el("input", { id: "install-path", type: "text", autocomplete: "off", spellcheck: false, placeholder: "D:\\Games\\MOHAA", value: view.manualPath, oninput: (event) => { view.manualPath = event.target.value; }, onkeydown: (event) => { if (event.key === "Enter") void check(view.manualPath, render); } })),
    el("button", { className: "btn", onclick: () => void browse(render) }, "Browse…")),
    el("button", { className: "btn btn--primary btn--block", onclick: () => void check(view.manualPath, render), disabled: view.manualPath.trim() === "" }, "Use this folder"));
}

async function browse(render) {
  view.error = null; render();
  try { const folder = await pickInstallFolder(); if (folder) { view.manualPath = folder; await check(folder, render); } }
  catch (error) { view.error = errorText(error); render(); }
}

async function check(path, render) {
  view.busy = true; view.error = null; view.message = "Reading."; render();
  try {
    const install = await detectInstall(path);
    if (install) { adoptCandidate(install); view.message = "Read from the game files on disk."; await loadOverview(install, render); }
    else { resetCandidate(); view.message = "That folder holds no Medal of Honor game files."; }
  } catch (error) { view.error = errorText(error); view.message = "That folder could not be read."; }
  finally { view.busy = false; render(); }
}

export async function autoDetect(render, onReady, { skipConfirmation = true } = {}) {
  view.busy = true; view.error = null; resetCandidate(); view.eyebrow = skipConfirmation ? "First run" : "Game folder"; render();
  try {
    const remembered = state.rememberedInstall;
    let install = remembered ? await safeDetect(remembered) : null;
    install ??= await detectInstall(null);
    if (install) {
      adoptCandidate(install); view.manualPath = displayPath(install.root); view.message = "Read from the game files on disk.";
      await loadOverview(install, render);
      if (skipConfirmation && remembered && install.root === remembered && selectedAvailable()) await accept(install, onReady, render);
    } else view.message = "Nothing was found automatically. Pick the folder once and Reveille remembers it.";
  } catch (error) { view.error = errorText(error); view.message = "Detection failed. Pick the folder instead."; }
  finally { view.busy = false; render(); }
}

async function safeDetect(path) { try { return await detectInstall(path); } catch { return null; } }
async function accept(install, onReady, render) {
  if (!selectedAvailable()) return;
  view.busy = true; view.error = null; render();
  try {
    view.overview = await selectEngine(install.root, view.selected);
    state.install = install; state.engine = view.selected;
    state.game = view.game ?? defaultGame(install);
    rememberInstall(install.root);
    rememberEngine(install.root, view.selected); rememberGame(install.root, state.game);
    notify(); onReady();
  } catch (error) { view.error = errorText(error); }
  finally { view.busy = false; render(); }
}
function resetCandidate() {
  loadToken += 1; view.candidate = null; view.overview = null; view.selected = null; view.game = null;
  view.openStatus = null; view.openError = null; view.result = null;
}
