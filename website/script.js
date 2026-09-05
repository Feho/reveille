// SPDX-License-Identifier: GPL-2.0-only

const header = document.querySelector("[data-header]");
const nav = document.querySelector("[data-nav]");
const navToggle = document.querySelector("[data-nav-toggle]");

function updateHeader() {
  header?.classList.toggle("is-scrolled", window.scrollY > 16);
}

function closeNavigation() {
  if (!(nav instanceof HTMLElement) || !(navToggle instanceof HTMLButtonElement)) return;
  nav.classList.remove("is-open");
  navToggle.setAttribute("aria-expanded", "false");
  const label = navToggle.querySelector(".sr-only");
  if (label) label.textContent = "Open menu";
}

function toggleNavigation() {
  if (!(nav instanceof HTMLElement) || !(navToggle instanceof HTMLButtonElement)) return;
  const isOpen = navToggle.getAttribute("aria-expanded") === "true";
  nav.classList.toggle("is-open", !isOpen);
  navToggle.setAttribute("aria-expanded", String(!isOpen));
  const label = navToggle.querySelector(".sr-only");
  if (label) label.textContent = isOpen ? "Open menu" : "Close menu";
}

updateHeader();
window.addEventListener("scroll", updateHeader, { passive: true });
navToggle?.addEventListener("click", toggleNavigation);
nav?.addEventListener("click", (event) => {
  if (event.target instanceof HTMLAnchorElement) closeNavigation();
});

window.addEventListener("resize", () => {
  if (window.innerWidth > 900) closeNavigation();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeNavigation();
});

// --- Latest release ------------------------------------------------------
//
// Nothing on this page hard-codes a version. The markup ships a link to
// `releases/latest`, which is correct whatever the current release is, and the
// fetch below upgrades it to the installer asset and states the version and
// size. A rate-limited, offline or failed request leaves the honest fallback
// in place rather than showing a version that may not be the current one.

const RELEASE_ENDPOINT = "https://api.github.com/repos/MOHCentral/reveille/releases/latest";
const INSTALLER_SUFFIX = "x64-setup.exe";

// Mebibytes, because that is what the GitHub release page the visitor may compare against shows.
function formatSize(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return null;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

async function showLatestRelease() {
  const response = await fetch(RELEASE_ENDPOINT, {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!response.ok) return;

  const release = await response.json();
  // `tag_name` is `v0.1.3`; the chip and the button subtitle both want it as written.
  const tag = typeof release?.tag_name === "string" ? release.tag_name : null;
  if (!tag) return;

  const installer = Array.isArray(release.assets)
    ? release.assets.find((asset) => typeof asset?.name === "string" && asset.name.endsWith(INSTALLER_SUFFIX))
    : undefined;

  const chip = document.querySelector("[data-release-version]");
  if (chip) chip.textContent = `${tag} · Windows`;

  const size = formatSize(installer?.size);
  const detail = [tag, "64-bit", size].filter(Boolean).join(" · ");
  for (const element of document.querySelectorAll("[data-release-detail]")) {
    element.textContent = detail;
  }

  if (typeof installer?.browser_download_url !== "string") return;
  for (const element of document.querySelectorAll("[data-release-download]")) {
    element.setAttribute("href", installer.browser_download_url);
  }
}

showLatestRelease().catch(() => {
  // The fallback markup already links to the latest release page.
});
