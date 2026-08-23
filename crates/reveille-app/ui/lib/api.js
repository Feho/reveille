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
//   browse_servers(path)                       -> BrowserPayload
//   cancel_browse()                            -> void
//   preview_join(path, address)                -> JoinPreview
//   install_and_launch(path, address, selectedCandidateIds, acceptIncomplete) -> JoinResult
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

export const openMohaaStatus = (path, channel) =>
  invoke("openmohaa_status", { path, channel });

export const installOpenMohaa = (path, offerId) =>
  invoke("install_openmohaa", { path, offerId });

export const cancelOpenMohaaInstall = () => invoke("cancel_openmohaa_install");

export const pickInstallFolder = () => invoke("pick_install_folder");

export const browseServers = (path) => invoke("browse_servers", { path });

export const cancelBrowse = () => invoke("cancel_browse");

export const previewJoin = (path, address) => invoke("preview_join", { path, address });

export const installAndLaunch = (path, address, selectedCandidateIds, acceptIncomplete) =>
  invoke("install_and_launch", { path, address, selectedCandidateIds, acceptIncomplete });

export const onBrowseProgress = (handler) => on("reveille://browse", handler);
export const onPreviewProgress = (handler) => on("reveille://preview", handler);
export const onInstallProgress = (handler) => on("reveille://install", handler);
export const onOpenMohaaInstallProgress = (handler) =>
  on("reveille://openmohaa-install", handler);

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
