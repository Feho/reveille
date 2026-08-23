// SPDX-License-Identifier: GPL-2.0-only

import { el, fill } from "../lib/dom.js";
import {
  cancelOpenMohaaInstall, cancelRebornInstall, detectInstall, engineOverview, errorText,
  installOpenMohaa, installReborn, onOpenMohaaInstallProgress, onRebornInstallProgress,
  openMohaaStatus, pickInstallFolder, selectEngine,
} from "../lib/api.js";
import { bytes, displayPath } from "../lib/format.js";
import { notify, recallEngine, rememberEngine, rememberInstall, state } from "../lib/store.js";

const PRODUCT_NAMES = { allied_assault: "Allied Assault", spearhead: "Spearhead", breakthrough: "Breakthrough" };
const DESCRIPTIONS = {
  openmohaa: "Modern rebuilt game program.",
  reborn: "Classic game program with community fixes.",
  original: "Existing game program without community-engine updates.",
};
const LABELS = { openmohaa: "OpenMoHAA", reborn: "Reborn", original: "Original game" };

const view = {
  candidate: null, eyebrow: "First run", message: "Checking the usual locations.", busy: true,
  error: null, manualPath: "", overview: null, selected: null, channel: "stable",
  openStatus: null, openError: null, installing: null, stopping: false, progress: null, result: null,
};
let loadToken = 0;

export function setupView(root, { onReady }) {
  const render = () => fill(root, card(render, onReady));
  void onOpenMohaaInstallProgress((progress) => progressFor("openmohaa", progress, render));
  void onRebornInstallProgress((progress) => progressFor("reborn", progress, render));
  render();
  return { render };
}

function progressFor(engine, progress, render) {
  if (view.installing !== engine) return;
  view.progress = progress;
  render();
}

function card(render, onReady) {
  const available = selectedAvailable();
  return el("div", { className: "setup" }, el("div", { className: "setup__card" },
    el("div", { className: "setup__brand" }, el("span", { className: "wordmark" }, "Reveille"), el("span", { className: "label" }, view.eyebrow)),
    el("h1", { className: "setup__title" }, view.busy ? "Looking for Allied Assault" : view.candidate ? "How do you want to run the game?" : "Show Reveille your game"),
    el("p", { className: "setup__lede" }, view.message),
    view.candidate && foundBlock(view.candidate),
    view.candidate && engineChoices(view.candidate, render),
    !view.busy && !view.candidate && manualBlock(render),
    view.candidate && el("div", { className: "actions__row" },
      el("button", { className: "btn btn--primary btn--block", disabled: view.busy || Boolean(view.installing) || !available, onclick: () => void accept(view.candidate, onReady, render) }, available ? "Continue to servers" : "Choose an available engine"),
      el("button", { className: "btn btn--ghost", disabled: view.busy || Boolean(view.installing), onclick: () => { resetCandidate(); view.message = "Pick your game folder."; render(); } }, "Choose another folder")),
    view.error && el("p", { className: "error", role: "alert" }, view.error),
  ));
}

function foundBlock(install) {
  const products = (install.products ?? []).map((product) => PRODUCT_NAMES[product] ?? product);
  return el("div", { className: "setup__found" }, el("p", { className: "setup__path" }, displayPath(install.root)), el("p", { className: "quiet" }, products.length ? products.join(" · ") : "No recognised game data"));
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
        el("option", { value: "stable", selected: view.channel === "stable" }, "Stable"), el("option", { value: "dev", selected: view.channel === "dev" }, "Preview — less tested"))),
    view.openError && el("span", { className: "note note--bad", title: view.openError }, "Release details could not be checked right now."),
    status?.availability === "unsupported" && el("span", { className: "note note--bad" }, "OpenMoHAA is unavailable for this computer."),
    status?.availability === "available" && el("span", { className: "kv-line" }, el("span", null, "Version"), el("strong", null, view.channel === "dev" ? "Newest preview" : status.package.version)),
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
  view.candidate = (await detectInstall(install.root)) ?? install; render();
}
function finishInstall(render) { view.installing = null; view.stopping = false; view.progress = null; render(); }
async function stopInstall(render) {
  view.stopping = true; render();
  try { await (view.installing === "reborn" ? cancelRebornInstall() : cancelOpenMohaaInstall()); }
  catch (error) { view.error = errorText(error); view.stopping = false; render(); }
}

function manualBlock(render) {
  return el("div", { className: "stack" }, el("div", { className: "setup__row" },
    el("label", { className: "field", for: "install-path" }, el("input", { id: "install-path", type: "text", autocomplete: "off", spellcheck: false, placeholder: "D:\\Games\\MOHAA", value: view.manualPath, oninput: (event) => { view.manualPath = event.target.value; }, onkeydown: (event) => { if (event.key === "Enter") void check(view.manualPath, render); } })),
    el("button", { className: "btn", onclick: () => void browse(render) }, "Browse…")),
    el("button", { className: "btn btn--primary btn--block", onclick: () => void check(view.manualPath, render), disabled: view.manualPath.trim() === "" }, "Check this folder"));
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
    if (install) { view.candidate = install; view.message = "Read from the game files on disk."; await loadOverview(install, render); }
    else { resetCandidate(); view.message = "No Medal of Honor installation there."; }
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
      view.candidate = install; view.manualPath = displayPath(install.root); view.message = "Read from the game files on disk.";
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
    state.install = install; state.engine = view.selected; rememberInstall(install.root);
    rememberEngine(install.root, view.selected); notify(); onReady();
  } catch (error) { view.error = errorText(error); }
  finally { view.busy = false; render(); }
}
function resetCandidate() {
  loadToken += 1; view.candidate = null; view.overview = null; view.selected = null;
  view.openStatus = null; view.openError = null; view.result = null;
}
