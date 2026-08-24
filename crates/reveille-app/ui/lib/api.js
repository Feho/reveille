// SPDX-License-Identifier: GPL-2.0-only

// The only module that knows the Rust contract. Everything else speaks in terms
// of these functions, so a change to a command signature has exactly one place
// to land.
//
// Commands (crates/reveille-app/src/main.rs):
//   detect_install(selectedPath?)              -> Installation | null
//   openmohaa_status(path, channel)            -> OpenMohaaStatus
//   install_openmohaa(path, offerId)           -> OpenMohaaInstallResult
//   cancel_openmohaa_install()                 -> void
//   pick_install_folder()                      -> string | null
//   engine_overview(path)                      -> EngineOverview
//   select_engine(path, engine)                -> EngineOverview
//   install_reborn(path)                       -> RebornInstallResult
//   browse_servers(path, engine)               -> BrowserPayload
//   cancel_browse()                            -> void
//   check_server(path, address, queryPort, engine) -> CheckResult
//   preview_join(path, address, engine)         -> JoinPreview
//   install_and_launch(path, address, engine, selectedCandidateIds, acceptIncomplete) -> JoinResult
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

export const browseServers = (path, engine) => invoke("browse_servers", { path, engine });

export const cancelBrowse = () => invoke("cancel_browse");

/**
 * Probe one remembered server directly. Resolves either way: a server that did not answer comes
 * back as `{ row: null, non_result }`, never as a rejection.
 */
export const checkServer = (path, address, queryPort, engine) =>
  invoke("check_server", { path, address, queryPort, engine });

export const previewJoin = (path, address, engine) => invoke("preview_join", { path, address, engine });

export const installAndLaunch = (path, address, engine, selectedCandidateIds, acceptIncomplete) =>
  invoke("install_and_launch", { path, address, engine, selectedCandidateIds, acceptIncomplete });

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
