// SPDX-License-Identifier: GPL-2.0-only

// One context menu, opened at a point and closed by anything.
//
// It exists because the alternative was WebView2's own menu — Back, Reload, Inspect — on every
// right-click, which is the loudest "web page in a costume" tell a Tauri app can produce and sits
// exactly where the native convention for this kind of application puts bookmarking
// (docs/ux-standards.md §7.3, docs/design-review.md F22).
//
// Two rules hold for everything put in it:
//
//   * Nothing is reachable **only** here. A context menu that owns an action is a trap for anyone
//     who does not think to right-click, which includes most beginners and every keyboard.
//   * It opens from the keyboard as well — Shift+F10 and the Menu key — anchored to the focused
//     element rather than to a pointer that was never there.

import { el, fill } from "./dom.js";

let open = null;

/**
 * Open a menu.
 *
 * `items` are `{ label, hint?, disabled?, onSelect }`. `event` supplies the pointer position when
 * there is one; `anchor` is the element the menu belongs to, used to place it for a keyboard and
 * to take focus back when the menu closes.
 */
export function openMenu(items, event, anchor = null) {
  closeMenu();
  if (!items.length) return;

  const list = el("div", { className: "menu", role: "menu", tabIndex: -1 });
  const buttons = items.map((item) =>
    el(
      "button",
      {
        type: "button",
        className: "menu__item",
        role: "menuitem",
        tabIndex: -1,
        // `aria-disabled`, not `disabled`, for the same reason as everywhere else in this
        // interface: a disabled element cannot hold focus, and a menu the arrow keys skip past
        // silently is worse than one that says why an entry does nothing.
        "aria-disabled": item.disabled ? "true" : null,
        onclick: () => {
          if (item.disabled) return;
          closeMenu();
          item.onSelect();
        },
      },
      el("span", { className: "menu__label" }, item.label),
      item.hint ? el("span", { className: "menu__hint" }, item.hint) : null,
    ),
  );
  fill(list, ...buttons);
  document.body.append(list);

  place(list, event, anchor);
  const first = buttons.find((button) => button.getAttribute("aria-disabled") !== "true");
  (first ?? list).focus();

  const onKey = (keyEvent) => {
    const at = buttons.indexOf(document.activeElement);
    if (keyEvent.key === "Escape") {
      keyEvent.preventDefault();
      closeMenu();
    } else if (keyEvent.key === "ArrowDown") {
      keyEvent.preventDefault();
      buttons[Math.min(at + 1, buttons.length - 1)]?.focus();
    } else if (keyEvent.key === "ArrowUp") {
      keyEvent.preventDefault();
      buttons[Math.max(at - 1, 0)]?.focus();
    } else if (keyEvent.key === "Home") {
      keyEvent.preventDefault();
      buttons[0]?.focus();
    } else if (keyEvent.key === "End") {
      keyEvent.preventDefault();
      buttons.at(-1)?.focus();
    } else if (keyEvent.key === "Tab") {
      // A menu is modal for the keyboard while it is up; Tab dismisses rather than escaping into
      // the page behind it with the menu still drawn.
      closeMenu();
    }
  };
  const onAway = (awayEvent) => {
    if (!list.contains(awayEvent.target)) closeMenu();
  };

  list.addEventListener("keydown", onKey);
  // Capture, so a click anywhere closes the menu before the thing under it reacts.
  document.addEventListener("pointerdown", onAway, true);
  window.addEventListener("blur", closeMenu);
  window.addEventListener("resize", closeMenu);

  open = {
    list,
    anchor,
    teardown: () => {
      list.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onAway, true);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("resize", closeMenu);
    },
  };
}

export function closeMenu() {
  if (!open) return;
  const { list, anchor, teardown } = open;
  open = null;
  teardown();
  const returning = list.contains(document.activeElement);
  list.remove();
  // Focus goes back where it came from, but only when the menu actually held it: closing because
  // the player clicked something else must not drag the caret away from what they clicked.
  if (returning) anchor?.focus?.();
}

export function menuIsOpen() {
  return open !== null;
}

/**
 * Put the menu where it was asked for, and keep it on screen.
 *
 * A pointer event gives a point. A keyboard event gives none, so the menu hangs under the left
 * edge of the element it belongs to — which is where Windows puts it for Shift+F10.
 */
function place(list, event, anchor) {
  const margin = 8;
  const { offsetWidth: width, offsetHeight: height } = list;
  let left;
  let top;
  if (typeof event?.clientX === "number" && (event.clientX > 0 || event.clientY > 0)) {
    left = event.clientX;
    top = event.clientY;
  } else {
    const box = anchor?.getBoundingClientRect?.() ?? { left: margin, bottom: margin };
    left = box.left + margin;
    top = box.bottom;
  }
  list.style.left = `${Math.max(margin, Math.min(left, window.innerWidth - width - margin))}px`;
  list.style.top = `${Math.max(margin, Math.min(top, window.innerHeight - height - margin))}px`;
}
