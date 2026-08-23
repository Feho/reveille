// SPDX-License-Identifier: GPL-2.0-only

// First run. Shown only while no installation is resolved.
//
// This is not a welcome page. It answers one question — where is the game — and
// says how confidently it was answered, because "identified from a build string"
// and "matched a known binary hash" are different claims and the interface should
// not blur them.

import { el, fill } from "../lib/dom.js";
import {
  cancelOpenMohaaInstall,
  detectInstall,
  errorText,
  installOpenMohaa,
  onOpenMohaaInstallProgress,
  openMohaaStatus,
  pickInstallFolder,
} from "../lib/api.js";
import { bytes, displayPath } from "../lib/format.js";
import { notify, rememberInstall, state } from "../lib/store.js";

const PRODUCT_NAMES = {
  allied_assault: "Allied Assault",
  spearhead: "Spearhead",
  breakthrough: "Breakthrough",
};

const RELEASE_CHOICES = {
  stable: "Recommended — best for most players",
  dev: "Preview — newest changes",
};

const IDENTIFICATION = {
  known_binary_hashes: {
    chip: "Verified",
    kind: "chip--ok",
    detail: "The game binary matches a build Reveille has fingerprints for.",
  },
  recognized_binary_unknown_hashes: {
    chip: "Recognised",
    kind: "chip--brass",
    detail:
      "The binary is a Medal of Honor client, but this exact build is not in Reveille's corpus. Identified by name, not by hash.",
  },
  data_directories_only: {
    chip: "Data only",
    kind: "chip--plain",
    detail:
      "The game data is here but no client executable was found. Reveille can index maps; launching needs a client.",
  },
};

const view = {
  /** Local, pre-commit state: what detection found before the player accepts it. */
  candidate: null,
  /** Why this screen is showing: boot, or the player asking to change folders. */
  eyebrow: "First run",
  message: "Checking the usual locations.",
  busy: true,
  error: null,
  manualPath: "",
  engine: engineState(),
};

let engineToken = 0;

export function setupView(root, { onReady }) {
  const render = () => fill(root, card(render, onReady));
  void onOpenMohaaInstallProgress((progress) => {
    if (!view.engine.installing) return;
    view.engine.progress = progress;
    render();
  });
  render();
  return { render };
}

function card(render, onReady) {
  return el(
    "div",
    { className: "setup" },
    el(
      "div",
      { className: "setup__card" },
      el(
        "div",
        { className: "setup__brand" },
        el("span", { className: "wordmark" }, "Reveille"),
        el("span", { className: "label" }, view.eyebrow),
      ),
      el("h1", { className: "setup__title" }, title()),
      el("p", { className: "setup__lede" }, view.message),
      view.candidate && foundBlock(view.candidate),
      view.candidate && openMohaaBlock(view.candidate, render),
      !view.busy && !view.candidate && manualBlock(render),
      view.candidate &&
        el(
          "div",
          { className: "actions__row" },
          el(
            "button",
            {
              className: "btn btn--primary btn--block",
              disabled: view.engine.installing || view.busy,
              onclick: () => accept(view.candidate, onReady),
            },
            "Continue to servers",
          ),
          el(
            "button",
            {
              className: "btn btn--ghost",
              disabled: view.engine.installing || view.busy,
              onclick: () => {
                view.candidate = null;
                resetEngine();
                view.message = "Pick your game folder.";
                render();
              },
            },
            "Choose another folder",
          ),
        ),
      view.error && el("p", { className: "error", role: "alert" }, view.error),
    ),
  );
}

function title() {
  if (view.busy) return "Looking for Allied Assault";
  if (view.candidate) return view.eyebrow === "First run" ? "Found your game" : "Your game folder";
  return "Show Reveille your game";
}

function foundBlock(install) {
  const identification =
    IDENTIFICATION[install.identification?.method] ?? IDENTIFICATION.data_directories_only;
  const products = (install.products ?? []).map((product) => PRODUCT_NAMES[product] ?? product);
  const named = (install.binaries ?? []).find((binary) => binary.known_version);
  return el(
    "div",
    { className: "setup__found" },
    el(
      "div",
      { className: "row-between" },
      el("p", { className: "setup__path" }, displayPath(install.root)),
      el(
        "span",
        { className: `chip ${identification.kind}`, title: identification.detail },
        identification.chip,
      ),
    ),
    el(
      "dl",
      { className: "kv" },
      el("dt", null, "Contains"),
      el("dd", null, products.length ? products.join(" · ") : "no recognised game data"),
      el("dt", null, "Client"),
      el(
        "dd",
        null,
        install.binaries?.length
          ? el(
              "span",
              null,
              el("strong", null, named?.known_version ?? "unrecognised build"),
              ` · ${install.binaries.length === 1 ? "1 binary" : `${install.binaries.length} binaries`}`,
            )
          : "none found",
      ),
    ),
    products.length > 1 &&
      el("p", { className: "quiet" }, "Reveille v1 handles Allied Assault only."),
  );
}

function engineState(overrides = {}) {
  return {
    channel: "stable",
    loading: false,
    status: null,
    error: null,
    installing: false,
    stopping: false,
    progress: null,
    result: null,
    ...overrides,
  };
}

function resetEngine() {
  engineToken += 1;
  view.engine = engineState();
}

function openMohaaBlock(install, render) {
  const engine = view.engine;
  if (engine.loading) {
    return el(
      "section",
      { className: "setup__found setup__engine", "aria-live": "polite" },
      el("div", { className: "row-between" }, el("h2", { className: "heading-sm" }, "OpenMoHAA")),
      channelChoice(install, render, engine.channel, true),
      el("div", { className: "meter meter--indeterminate" }, el("span", { className: "meter__fill" })),
      el(
        "p",
        { className: "quiet" },
        engine.channel === "dev" ? "Checking the newest preview." : "Checking the recommended version.",
      ),
    );
  }
  if (engine.error) {
    return el(
      "section",
      { className: "setup__found setup__engine" },
      el("div", { className: "row-between" }, el("h2", { className: "heading-sm" }, "OpenMoHAA")),
      channelChoice(install, render, engine.channel),
      el(
        "p",
        { className: "error", role: "alert", title: engine.error.detail },
        engine.error.message,
      ),
      el(
        "button",
        { className: "btn btn--ghost btn--sm", onclick: () => refreshInstall(install, render) },
        "Check again",
      ),
    );
  }
  const status = engine.status;
  if (!status) return false;
  if (status.availability === "unsupported") {
    return el(
      "section",
      { className: "setup__found setup__engine" },
      el("div", { className: "row-between" }, el("h2", { className: "heading-sm" }, "OpenMoHAA")),
      channelChoice(install, render, engine.channel),
      el(
        "p",
        {
          className: "note",
          title: `No supported release for ${status.os}/${status.architecture}.`,
        },
        "OpenMoHAA is not available for this kind of computer.",
      ),
    );
  }

  const buildState = status.installed_build.state;
  const installed = buildState !== "absent";
  const current = buildState === "current";
  const replacementPrimary = buildState === "absent" || buildState === "known_other";
  const activityBlocksReplacement =
    installed && status.activity.state !== "confirmed_stopped";
  const packageInfo = status.package;
  return el(
    "section",
    { className: "setup__found setup__engine", "aria-live": "polite" },
    el(
      "div",
      { className: "row-between" },
      el("h2", { className: "heading-sm" }, "OpenMoHAA"),
      el(
        "span",
        {
          className: `chip ${current ? "chip--ok" : installed ? "chip--brass" : "chip--plain"}`,
        },
        current ? "Up to date" : installed ? "Installed" : "Not installed",
      ),
    ),
    el(
      "p",
      { className: "quiet" },
      "A modern replacement for the original game program. It uses the game files you already own.",
    ),
    channelChoice(install, render, engine.channel),
    engine.channel === "dev" &&
      el(
        "p",
        { className: "note" },
        "Preview builds include the newest changes, but they have had less testing.",
      ),
    el(
      "dl",
      { className: "kv" },
      el("dt", null, engine.channel === "dev" ? "Preview build" : "Recommended"),
      el(
        "dd",
        { title: engine.channel === "dev" ? packageInfo.version : null },
        el("strong", null, engine.channel === "dev" ? "Newest available" : packageInfo.version),
      ),
      el("dt", null, "Download size"),
      el("dd", { title: packageInfo.asset_name }, bytes(packageInfo.size)),
      el("dt", null, "Status"),
      el("dd", null, clientActivity(status)),
    ),
    installedBuildNote(status, engine.channel, packageInfo),
    el(
      "p",
      {
        className: "note",
        title: `The release page publishes this file check: ${packageInfo.digest}`,
      },
      "Reveille checks that the download arrived intact before installing it.",
    ),
    engine.installing && engineProgress(engine),
    engine.result && engineResult(engine.result),
    activityBlocksReplacement &&
      el(
        "p",
        { className: "note note--bad" },
        status.activity.state === "running"
          ? closeProgramsMessage(status.activity.running)
          : "Reveille could not check whether an OpenMoHAA game, server, or launcher is open. Close any that are open, then press Refresh.",
      ),
    engine.installing
      ? el(
          "button",
          {
            className: "btn btn--ghost btn--block",
            disabled: engine.stopping,
            onclick: () => stopOpenMoHaa(render),
          },
          engine.stopping ? "Stopping…" : "Stop download",
        )
      : el(
          "div",
          { className: "actions__row" },
          !replacementPrimary && refreshEngineButton(install, render),
          el(
            "button",
            {
              className: `btn ${replacementPrimary ? "btn--primary" : "btn--ghost btn--sm"} btn--block`,
              disabled: activityBlocksReplacement,
              onclick: () => installEngine(install, render),
            },
            engineActionLabel(buildState, engine.channel),
          ),
          replacementPrimary && refreshEngineButton(install, render),
        ),
  );
}

function clientActivity(status) {
  const buildState = status.installed_build.state;
  if (buildState === "absent") return "Not installed";
  switch (status.activity.state) {
    case "confirmed_stopped":
      if (buildState === "current") return "Installed · up to date";
      if (buildState === "known_other") {
        return status.installed_build.channel === "dev"
          ? "Installed · another preview build"
          : `Installed · ${status.installed_build.version}`;
      }
      return "Installed · version unknown";
    case "running":
      return `Installed · ${runningProgramsLabel(status.activity.running)} open`;
    default:
      return "Installed · could not check whether it is open";
  }
}

function installedBuildNote(status, selectedChannel, packageInfo) {
  const installed = status.installed_build;
  switch (installed.state) {
    case "current":
    case "absent":
      return false;
    case "known_other":
      if (installed.channel === "dev") {
        return el(
          "p",
          { className: "quiet", title: installed.version },
          selectedChannel === "dev"
            ? "A different preview build is installed. You can switch to the newest preview."
            : "A preview build is installed. You can switch to the recommended version.",
        );
      }
      return el(
        "p",
        { className: "quiet" },
        selectedChannel === "dev"
          ? `${installed.version} is installed. You can switch to the newest preview.`
          : `${installed.version} is installed. The recommended version is ${packageInfo.version}.`,
      );
    default:
      return el(
        "p",
        { className: "quiet" },
        "OpenMoHAA is installed, but Reveille cannot identify this copy. Replacing it is optional.",
      );
  }
}

function engineActionLabel(buildState, channel) {
  if (buildState === "absent") {
    return channel === "dev" ? "Install preview build" : "Install OpenMoHAA";
  }
  if (buildState === "current") return "Reinstall this version";
  if (buildState === "known_other") {
    return channel === "dev" ? "Switch to newest preview" : "Switch to recommended version";
  }
  return channel === "dev" ? "Replace with preview build" : "Replace with recommended version";
}

function refreshEngineButton(install, render) {
  return el(
    "button",
    {
      className: "btn btn--ghost",
      onclick: () => refreshInstall(install, render),
    },
    "Refresh",
  );
}

function channelChoice(install, render, selected, disabled = false) {
  return el(
    "label",
    { className: "engine-choice" },
    el("span", { className: "engine-choice__label" }, "Choose a version"),
    el(
      "select",
      {
        disabled,
        value: selected,
        onchange: (event) => loadOpenMohaa(install, render, event.target.value),
      },
      Object.entries(RELEASE_CHOICES).map(([value, label]) =>
        el("option", { value, selected: value === selected }, label),
      ),
    ),
  );
}

function runningProgramsLabel(programs) {
  const labels = new Set(programs);
  if (labels.size > 1) return "OpenMoHAA programs are";
  if (labels.has("dedicated_server")) return "server is";
  if (labels.has("launcher")) return "launcher is";
  return "game is";
}

function closeProgramsMessage(programs) {
  const labels = new Set(programs);
  if (labels.size > 1) {
    const names = [];
    if (labels.has("game")) names.push("game");
    if (labels.has("dedicated_server")) names.push("server");
    if (labels.has("launcher")) names.push("launcher");
    const last = names.pop();
    return `The OpenMoHAA ${names.join(", ")} and ${last} are open. Close them before updating.`;
  }
  if (labels.has("dedicated_server")) {
    return "An OpenMoHAA server is open. Close it before updating.";
  }
  if (labels.has("launcher")) {
    return "An OpenMoHAA launcher is open. Close it before updating.";
  }
  return "The OpenMoHAA game is open. Close it before updating.";
}

function engineProgress(engine) {
  const received = engine.progress?.received ?? 0;
  const total = engine.progress?.total ?? null;
  const percent = total ? Math.min(100, (received / total) * 100) : null;
  return el(
    "div",
    { className: "stack--tight" },
    el(
      "div",
      { className: `meter ${percent === null ? "meter--indeterminate" : ""}` },
      el("span", {
        className: "meter__fill",
        style: percent === null ? null : `width: ${percent}%`,
      }),
    ),
    el(
      "p",
      { className: "quiet data" },
      total ? `${bytes(received)} of ${bytes(total)}` : "Preparing download",
    ),
  );
}

function engineResult(result) {
  if (result.cancelled) return el("p", { className: "note" }, "Download stopped. No files changed.");
  const outcome = result.outcome;
  switch (outcome?.outcome) {
    case "installed":
      return el(
        "p",
        { className: "note note--brass" },
        result.package.channel === "dev"
          ? "The newest OpenMoHAA preview is ready."
          : `OpenMoHAA ${result.package.version} is ready.`,
      );
    case "updated":
      return el(
        "p",
        { className: "note note--brass" },
        result.package.channel === "dev"
          ? "OpenMoHAA was replaced with the newest preview."
          : `OpenMoHAA was replaced with ${result.package.version}.`,
      );
    case "deferred":
      return el(
        "p",
        { className: "note note--bad" },
        outcome.reason === "client_running"
          ? `No files changed. ${closeProgramsMessage(result.activity?.running ?? [])}`
          : "No files changed because Reveille could not check whether an OpenMoHAA game, server, or launcher was open.",
      );
    default:
      return false;
  }
}

async function loadOpenMohaa(install, render, channel = view.engine.channel) {
  const token = ++engineToken;
  view.engine = engineState({ loading: true, channel });
  render();
  try {
    const status = await openMohaaStatus(install.root, channel);
    if (token !== engineToken) return;
    view.engine = engineState({ status, channel });
  } catch (error) {
    if (token !== engineToken) return;
    view.engine = engineState({ error: friendlyEngineError(error), channel });
  }
  render();
}

async function refreshInstall(install, render) {
  const channel = view.engine.channel;
  view.busy = true;
  view.error = null;
  render();
  try {
    const refreshed = await detectInstall(install.root);
    if (!refreshed) {
      throw new Error("The game folder is no longer available.");
    }
    view.candidate = refreshed;
    view.message = "Read from the game files on disk.";
    view.busy = false;
    await loadOpenMohaa(refreshed, render, channel);
  } catch (error) {
    view.candidate = null;
    resetEngine();
    view.message = "That game folder is no longer available. Choose the folder again.";
    view.error = errorText(error);
  } finally {
    view.busy = false;
    render();
  }
}

async function installEngine(install, render) {
  view.engine.installing = true;
  view.engine.stopping = false;
  view.engine.progress = null;
  view.engine.result = null;
  view.engine.error = null;
  render();
  try {
    const result = await installOpenMohaa(install.root, view.engine.status.package.offer_id);
    view.engine.result = result;
    if (result.outcome?.outcome === "installed" || result.outcome?.outcome === "updated") {
      view.engine.status.installed_build = result.installed_build;
      view.engine.status.activity = result.activity;
      view.engine.status.package = result.package;
      try {
        view.candidate = (await detectInstall(install.root)) ?? view.candidate;
      } catch {
        // The verified engine install succeeded; a display-only rescan cannot undo that result.
      }
    }
  } catch (error) {
    if (failureKind(error) === "cancelled") {
      view.engine.result = { cancelled: true };
    } else {
      view.engine.error = friendlyEngineError(error);
    }
  } finally {
    view.engine.installing = false;
    view.engine.stopping = false;
    render();
  }
}

async function stopOpenMoHaa(render) {
  view.engine.stopping = true;
  render();
  try {
    await cancelOpenMohaaInstall();
  } catch (error) {
    view.engine.stopping = false;
    view.engine.error = friendlyEngineError(error);
    render();
  }
}

// Keyed on OpenMohaaFailureKind (crates/reveille-app/src/main.rs), which classifies the Rust
// error once. Matching on formatted message text used to merge two different facts: a release
// that publishes no file check had never been downloaded at all, yet was reported as a download
// that did not arrive intact. Each message states only what was actually observed.
const ENGINE_ERRORS = {
  unreachable: "Reveille could not check for OpenMoHAA right now. Check your connection and try again.",
  no_asset_for_host: "OpenMoHAA is not available for this kind of computer.",
  release_metadata:
    "Reveille will not install this OpenMoHAA version: it does not come with the file check Reveille needs. Nothing was downloaded.",
  corrupt_download: "The download did not arrive intact, so nothing was installed. Try again.",
  archive_rejected: "The download was not something Reveille will install. Nothing was changed.",
  cancelled: "Download stopped. No files changed.",
  filesystem: "Reveille could not write to the game folder. Nothing was changed.",
  // `Internal` carries developer text — a busy install lock, an unreadable folder. Without an
  // entry here the raw Rust message became the body copy a beginner read.
  internal: "Reveille could not complete that. Nothing was changed.",
};

function failureKind(error) {
  return error && typeof error.kind === "string" ? error.kind : null;
}

/**
 * A message the player can act on, plus the original text as tooltip detail for diagnosis.
 *
 * An unclassified failure shows its own text rather than being assigned a cause it may not have.
 */
function friendlyEngineError(error) {
  const detail = failureKind(error) ? error.detail : errorText(error);
  return { message: ENGINE_ERRORS[failureKind(error)] ?? detail, detail };
}

function manualBlock(render) {
  return el(
    "div",
    { className: "stack" },
    el(
      "div",
      { className: "setup__row" },
      el(
        "label",
        { className: "field", for: "install-path" },
        el("input", {
          id: "install-path",
          type: "text",
          autocomplete: "off",
          spellcheck: false,
          placeholder: "D:\\Games\\MOHAA",
          value: view.manualPath,
          oninput: (event) => {
            view.manualPath = event.target.value;
          },
          onkeydown: (event) => {
            if (event.key === "Enter") check(view.manualPath, render);
          },
        }),
      ),
      el("button", { className: "btn", onclick: () => browse(render) }, "Browse…"),
    ),
    el(
      "div",
      { className: "actions__row" },
      el(
        "button",
        {
          className: "btn btn--primary btn--block",
          onclick: () => check(view.manualPath, render),
          disabled: view.manualPath.trim() === "",
        },
        "Check this folder",
      ),
    ),
    el(
      "p",
      { className: "quiet" },
      "The folder holding ",
      el("span", { className: "data" }, "main"),
      " and the game client.",
    ),
  );
}

async function browse(render) {
  view.error = null;
  render();
  try {
    const folder = await pickInstallFolder();
    if (!folder) return;
    view.manualPath = folder;
    await check(folder, render);
  } catch (error) {
    view.error = errorText(error);
    render();
  }
}

async function check(path, render) {
  view.busy = true;
  view.error = null;
  view.message = "Reading.";
  render();
  try {
    const install = await detectInstall(path);
    if (install) {
      view.candidate = install;
      view.message = "Read from the game files on disk.";
      void loadOpenMohaa(install, render);
    } else {
      resetEngine();
      view.message = "No Medal of Honor installation there.";
    }
  } catch (error) {
    view.error = errorText(error);
    view.message = "That folder could not be read.";
  } finally {
    view.busy = false;
    render();
  }
}

/**
 * Run detection: the remembered folder first, then the registry.
 *
 * `skipConfirmation` is true only at boot. A player who came here from the install
 * chip wants to change the folder, so re-accepting the remembered one on their
 * behalf would make the control do nothing at all.
 */
export async function autoDetect(render, onReady, { skipConfirmation = true } = {}) {
  view.busy = true;
  view.error = null;
  view.candidate = null;
  resetEngine();
  view.eyebrow = skipConfirmation ? "First run" : "Game folder";
  render();
  const remembered = state.rememberedInstall;
  try {
    let install = remembered ? await safeDetect(remembered) : null;
    install ??= await detectInstall(null);
    if (install) {
      // A remembered install is a decision the player already made; do not ask
      // again at boot.
      if (skipConfirmation && remembered && install.root === remembered) {
        accept(install, onReady);
        return;
      }
      view.candidate = install;
      view.message = "Read from the game files on disk.";
      view.manualPath = displayPath(install.root);
      void loadOpenMohaa(install, render);
    } else {
      view.message = skipConfirmation
        ? "Nothing was found automatically. Pick the folder once and Reveille remembers it."
        : "Pick your game folder.";
    }
  } catch (error) {
    view.error = errorText(error);
    view.message = "Detection failed. Pick the folder instead.";
  } finally {
    view.busy = false;
    render();
  }
}

async function safeDetect(path) {
  try {
    return await detectInstall(path);
  } catch {
    // A remembered folder that has since moved is not an error worth showing.
    return null;
  }
}

function accept(install, onReady) {
  state.install = install;
  rememberInstall(install.root);
  notify();
  onReady();
}
