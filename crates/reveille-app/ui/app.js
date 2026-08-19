// SPDX-License-Identifier: GPL-2.0-only

const invoke = window.__TAURI__.core.invoke;
const state = { install: null, servers: [], selected: null, preview: null };
const $ = (selector) => document.querySelector(selector);

function showScreen(name) {
  document.querySelectorAll(".screen").forEach((screen) => screen.classList.remove("active"));
  $(`#screen-${name}`).classList.add("active");
}

function setBusy(button, busy, label) {
  button.disabled = busy;
  if (label) button.textContent = busy ? label : button.dataset.label;
}

function rememberInstall(install) {
  state.install = install;
  localStorage.setItem("reveille-install", install.root);
  $("#install-pill").textContent = `Game found · ${displayPath(install.root)}`;
  $("#setup-title").textContent = "Allied Assault found";
  $("#setup-message").textContent = "Reveille checked the game files and knows where to put maps.";
  $("#found-path").textContent = displayPath(install.root);
  $("#install-found").classList.remove("hidden");
  $("#manual-path").classList.add("hidden");
  $("#continue-button").classList.remove("hidden");
}

async function detectInstall(path = null) {
  $("#setup-error").textContent = "";
  try {
    const install = await invoke("detect_install", { selectedPath: path });
    if (install) return rememberInstall(install);
    $("#setup-title").textContent = "Show Reveille your game";
    $("#setup-message").textContent = "Windows did not list an Allied Assault installation. You can point to it once.";
    $("#manual-path").classList.remove("hidden");
    $("#install-pill").textContent = "Game folder needed";
  } catch (error) {
    $("#setup-error").textContent = String(error);
    $("#manual-path").classList.remove("hidden");
  }
}

function stateInfo(value) {
  const kind = value?.state || "cant_tell";
  if (kind === "compatible") return { kind, label: "Compatible", icon: "✓", copy: "Nothing Reveille can check is wrong. The server still makes the final decision." };
  if (kind === "needs_maps") return { kind: "needs", label: `Needs ${value.count} maps`, icon: "↓", copy: "Reveille found the missing maps and can put them in the right place." };
  if (kind === "no_source") return { kind: "no-source", label: "No source", icon: "!", copy: "At least one required map could not be found. Reveille will not pretend this join is ready." };
  return { kind: "cant-tell", label: "Can't tell", icon: "?", copy: "This server does not publish its map list, so Reveille cannot check it in advance." };
}

function occupancy(server) {
  const clients = server.occupancy.clients_reported;
  const bots = server.occupancy.bots_reported;
  const capacity = server.client_capacity ?? "?";
  return `${clients ?? "?"} clients${bots > 0 ? ` (+${bots} bots)` : ""} · cap ${capacity}`;
}

function renderServers() {
  const query = $("#server-filter").value.trim().toLowerCase();
  const list = state.servers.filter((item) => item.server.hostname.toLowerCase().includes(query));
  $("#server-list").innerHTML = list.length ? list.map((item) => {
    const info = stateInfo(item.compatibility.state);
    const map = item.server.current_map || "Not published";
    return `<button class="server-row" data-address="${item.address}">
      <span class="server-name">${escapeHtml(item.server.hostname)}<small class="server-address">${item.address}</small></span>
      <span class="cell-muted">${occupancy(item.server)}</span>
      <span class="cell-muted">${escapeHtml(map)}</span>
      <span class="status ${info.kind}">${info.label}</span><span>›</span>
    </button>`;
  }).join("") : `<div class="empty">No server names match that search.</div>`;
  document.querySelectorAll(".server-row").forEach((row) => row.addEventListener("click", () => openJoin(row.dataset.address)));
}

async function refreshServers() {
  const button = $("#refresh-servers");
  button.dataset.label ||= button.textContent;
  setBusy(button, true, "Checking servers…");
  $("#browser-error").textContent = "";
  $("#server-list").innerHTML = `<div class="empty">Contacting the master list and checking each server…</div>`;
  try {
    const payload = await invoke("browse_servers", { path: state.install.root });
    state.servers = payload.servers;
    $("#browser-summary").textContent = `${payload.servers.length} servers answered · ${payload.recorded_non_results} did not answer and were skipped`;
    renderServers();
  } catch (error) {
    $("#browser-error").textContent = String(error);
    $("#server-list").innerHTML = `<div class="empty">Servers could not be refreshed. Your game files were not changed.</div>`;
  } finally { setBusy(button, false, "Checking servers…"); }
}

async function openJoin(address) {
  state.selected = state.servers.find((server) => server.address === address);
  showScreen("join");
  $("#join-server-name").textContent = state.selected.server.hostname;
  $("#join-server-meta").textContent = `${address} · ${occupancy(state.selected.server)}`;
  $("#state-card").className = "state-card loading";
  $("#state-title").textContent = "Checking your maps";
  $("#state-copy").textContent = "This usually takes a moment.";
  $("#map-actions").innerHTML = "";
  $("#play-button").classList.add("hidden");
  $("#join-error").textContent = "";
  try {
    state.preview = await invoke("preview_join", { path: state.install.root, address });
    renderPreview();
  } catch (error) { $("#join-error").textContent = String(error); }
}

function renderPreview() {
  const preview = state.preview;
  const info = stateInfo(preview.assessment.state);
  $("#state-card").className = `state-card ${info.kind}`;
  $("#state-icon").textContent = info.icon;
  $("#state-title").textContent = info.label;
  $("#state-copy").textContent = info.copy;
  $("#maps-check").innerHTML = `<span>${info.kind === "compatible" ? "✓" : "·"}</span> Maps used by this server`;
  const choices = [];
  for (const resolution of preview.catalogue?.resolutions || []) {
    if (resolution.outcome === "choice_required") {
      choices.push(`<div class="map-choice"><strong>${escapeHtml(resolution.wanted.name)} needs your choice</strong>${resolution.choices.map((choice) => `<label><input type="radio" name="choice-${escapeHtml(resolution.wanted.name)}" value="${choice.id}"> ${escapeHtml(choice.filename)}</label>`).join("")}</div>`);
    }
  }
  $("#map-actions").innerHTML = choices.join("");
  if (preview.used_home_fallback) {
    $("#fallback-note").textContent = `This install is protected. Maps will be kept in ${displayPath(preview.game_directory)}`;
    $("#fallback-note").classList.remove("hidden");
  } else { $("#fallback-note").classList.add("hidden"); }
  if (["compatible", "needs_maps", "cant_tell"].includes(preview.assessment.state.state)) {
    $("#play-button").textContent = preview.assessment.state.state === "needs_maps" ? "Get maps and play →" : "Play now →";
    $("#play-button").classList.remove("hidden");
  }
}

async function installAndLaunch() {
  const button = $("#play-button");
  button.dataset.label ||= button.textContent;
  setBusy(button, true, "Getting ready…");
  $("#join-error").textContent = "";
  const selectedCandidateIds = [...document.querySelectorAll("#map-actions input:checked")].map((input) => Number(input.value));
  try {
    const result = await invoke("install_and_launch", {
      path: state.install.root,
      address: state.selected.address,
      selectedCandidateIds,
      allowUnchecked: state.preview.assessment.state.state === "cant_tell"
    });
    if (result.process_id) {
      $("#state-title").textContent = "Game launched";
      $("#state-copy").textContent = "Allied Assault is connecting to the server now.";
      button.classList.add("hidden");
    } else {
      state.preview.assessment = result.assessment;
      renderPreview();
      $("#join-error").textContent = result.non_results.join(" · ") || "Some maps still need a source or a choice.";
    }
  } catch (error) { $("#join-error").textContent = String(error); }
  finally { setBusy(button, false, "Getting ready…"); }
}

function escapeHtml(value) { const element = document.createElement("span"); element.textContent = String(value); return element.innerHTML; }
function displayPath(value) { return String(value).replace(/^\\\\\?\\/, ""); }

$("#check-path").addEventListener("click", () => detectInstall($("#install-path").value));
$("#continue-button").addEventListener("click", () => { showScreen("browser"); refreshServers(); });
$("#refresh-servers").addEventListener("click", refreshServers);
$("#server-filter").addEventListener("input", renderServers);
$("#back-to-browser").addEventListener("click", () => showScreen("browser"));
$("#play-button").addEventListener("click", installAndLaunch);
document.querySelectorAll("[data-go='browser']").forEach((button) => button.addEventListener("click", () => state.install && showScreen("browser")));

const remembered = localStorage.getItem("reveille-install");
detectInstall(remembered);
