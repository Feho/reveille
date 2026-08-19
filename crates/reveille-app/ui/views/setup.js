// SPDX-License-Identifier: GPL-2.0-only

// First run. Shown only while no installation is resolved.
//
// This is not a welcome page. It answers one question — where is the game — and
// says how confidently it was answered, because "identified from a build string"
// and "matched a known binary hash" are different claims and the interface should
// not blur them.

import { el, fill } from "../lib/dom.js";
import { detectInstall, errorText, pickInstallFolder } from "../lib/api.js";
import { displayPath } from "../lib/format.js";
import { notify, rememberInstall, state } from "../lib/store.js";

const PRODUCT_NAMES = {
  allied_assault: "Allied Assault",
  spearhead: "Spearhead",
  breakthrough: "Breakthrough",
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
};

export function setupView(root, { onReady }) {
  const render = () => fill(root, card(render, onReady));
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
      !view.busy && !view.candidate && manualBlock(render),
      view.candidate &&
        el(
          "div",
          { className: "actions__row" },
          el(
            "button",
            {
              className: "btn btn--primary btn--block",
              onclick: () => accept(view.candidate, onReady),
            },
            "Use this install",
          ),
          el(
            "button",
            {
              className: "btn btn--ghost",
              onclick: () => {
                view.candidate = null;
                view.message = "Pick your game folder.";
                render();
              },
            },
            "Choose another",
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
    } else {
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
