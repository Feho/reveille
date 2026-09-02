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
