// SPDX-License-Identifier: GPL-2.0-only

// The only module that knows the Rust contract. Everything else speaks in terms
// of these functions, so a change to a command signature has exactly one place
// to land.
//
// Commands (crates/reveille-app/src/main.rs):
//   detect_install(selectedPath?)              -> Installation | null
//     Installation.products is what is on disk; Installation.playable is what can be run — an
//     expansion needs the base game underneath it (rules H13/H14). Offer `playable`.
//   openmohaa_status(path, channel)            -> OpenMohaaStatus
//   install_openmohaa(path, offerId)           -> OpenMohaaInstallResult
//   cancel_openmohaa_install()                 -> void
//   pick_install_folder()                      -> string | null
//   engine_overview(path)                      -> EngineOverview
//   select_engine(path, engine)                -> EngineOverview
//   install_reborn(path)                       -> RebornInstallResult
//   browse_servers(session)                    -> BrowserPayload
//   cancel_browse()                            -> void
//   check_server(session, address, queryPort)  -> CheckResult
//   preview_join(session, address)             -> JoinPreview
//   install_and_launch(session, address, selectedCandidateIds, acceptIncomplete) -> JoinResult
//
// A `session` is `{ path, engine, game }`: which game folder, which engine program, and which of
// the three games — Allied Assault, Spearhead or Breakthrough. Every server-facing command takes
// all three together, because a folder and an engine without a game names no search path.
//
// Events:
//   reveille://browse   BrowseProgress   { registered, inspected, probed, answered, non_results, row }
//   reveille://preview  PreviewProgress  { address, index, of, map }
//   reveille://install  InstallProgress  { map, filename, index, of, phase, ... }
//   reveille://openmohaa-install OpenMohaaInstallProgress { received, total }

const tauri = window.__TAURI__;
const invoke = tauri.core.invoke;
const listen = tauri.event.listen;

export const detectInstall = (selectedPath = null) => invoke("detect_install", { selectedPath });

export const engineOverview = (path, savedEngine = null) =>
  invoke("engine_overview", { path, savedEngine });
export const selectEngine = (path, engine) => invoke("select_engine", { path, engine });
export const installReborn = (path) => invoke("install_reborn", { path });
export const cancelRebornInstall = () => invoke("cancel_reborn_install");

export const openMohaaStatus = (path, channel) =>
  invoke("openmohaa_status", { path, channel });

export const installOpenMohaa = (path, offerId) =>
  invoke("install_openmohaa", { path, offerId });

export const cancelOpenMohaaInstall = () => invoke("cancel_openmohaa_install");

export const pickInstallFolder = () => invoke("pick_install_folder");

export const browseServers = (session) => invoke("browse_servers", { session });

export const cancelBrowse = () => invoke("cancel_browse");

/**
 * Probe one remembered server directly. Resolves either way: a server that did not answer comes
 * back as `{ row: null, non_result }`, and one that answered for another of the three games as
 * `{ row: null, other_game }`. Never a rejection.
 */
export const checkServer = (session, address, queryPort) =>
  invoke("check_server", { session, address, queryPort });

export const previewJoin = (session, address) => invoke("preview_join", { session, address });

export const installAndLaunch = (session, address, selectedCandidateIds, acceptIncomplete) =>
  invoke("install_and_launch", { session, address, selectedCandidateIds, acceptIncomplete });

export const onBrowseProgress = (handler) => on("reveille://browse", handler);
export const onPreviewProgress = (handler) => on("reveille://preview", handler);
export const onInstallProgress = (handler) => on("reveille://install", handler);
export const onOpenMohaaInstallProgress = (handler) =>
  on("reveille://openmohaa-install", handler);
export const onRebornInstallProgress = (handler) => on("reveille://reborn-install", handler);

function on(name, handler) {
  return listen(name, (event) => handler(event.payload));
}

/**
 * Commands reject with a plain string. Normalise so callers always get a string
 * to show, whatever the failure was.
 */
export function errorText(error) {
  if (typeof error === "string") return error;
  if (error && typeof error.message === "string") return error.message;
  return String(error);
}

/**
 * `browse_servers` is the one command that rejects with a classified failure rather than a string.
 *
 * `{ kind, detail }`, where `kind` is decided in Rust beside the errors it names — the shell must
 * never read a cause out of a formatted message, which is how "no internet" and "the master sent
 * nonsense" ended up as the same unreadable line (docs/design-review.md F6). Anything else that
 * reaches this is carried through as `internal` with its own message intact.
 */
export function browseFailure(error) {
  if (error && typeof error === "object" && typeof error.kind === "string") {
    return { kind: error.kind, detail: String(error.detail ?? "") };
  }
  return { kind: "internal", detail: errorText(error) };
}
