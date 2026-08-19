// SPDX-License-Identifier: GPL-2.0-only

// A minimal element builder. Everything in this interface is constructed rather
// than interpolated into innerHTML, so server hostnames and map names — which are
// arbitrary bytes from a third party — can never become markup.

/**
 * Build an element.
 *
 * `props` sets properties when the key exists on the element (className, value,
 * checked, textContent), attributes otherwise (aria-*, data-*, colspan), and
 * listeners for keys starting with "on".
 *
 * Children may be nodes, strings, numbers, or nested arrays. `null`, `undefined`
 * and `false` are skipped, so `condition && el(...)` reads naturally.
 */
export function el(tag, props = null, ...children) {
  const node = document.createElement(tag);
  if (props) {
    for (const [key, value] of Object.entries(props)) {
      if (value === null || value === undefined) continue;
      if (key.startsWith("on") && typeof value === "function") {
        node.addEventListener(key.slice(2).toLowerCase(), value);
      } else if (key === "dataset") {
        Object.assign(node.dataset, value);
      } else if (key in node && key !== "list") {
        node[key] = value;
      } else {
        node.setAttribute(key, value === true ? "" : String(value));
      }
    }
  }
  append(node, children);
  return node;
}

function append(node, children) {
  for (const child of children) {
    if (child === null || child === undefined || child === false || child === true) continue;
    if (Array.isArray(child)) append(node, child);
    else node.append(child instanceof Node ? child : String(child));
  }
}

/** Replace every child of `node` with `children`. */
export function fill(node, ...children) {
  node.replaceChildren();
  append(node, children);
  return node;
}

/** Document fragment, for returning several siblings from one function. */
export function frag(...children) {
  const fragment = document.createDocumentFragment();
  append(fragment, children);
  return fragment;
}

export const $ = (selector, root = document) => root.querySelector(selector);

/**
 * Re-render `region` without stealing the caret.
 *
 * Replacing a subtree detaches whatever had focus, which silently breaks typing
 * and arrow-key navigation. Elements that can survive a re-render carry a
 * `data-focus-key`; this records the focused key and text selection, runs the
 * repaint, and puts the caret back where the player left it.
 */
export function preserveFocus(region, repaint) {
  const active = document.activeElement;
  const key = region.contains(active) ? (active.dataset?.focusKey ?? null) : null;
  const start = key === null ? null : active.selectionStart;
  const end = key === null ? null : active.selectionEnd;

  repaint();

  if (key === null) return;
  const restored = region.querySelector(`[data-focus-key="${CSS.escape(key)}"]`);
  if (!restored) return;
  restored.focus();
  if (start !== null && typeof restored.setSelectionRange === "function") {
    try {
      restored.setSelectionRange(start, end);
    } catch {
      // Inputs whose type forbids selection ranges; the focus alone is enough.
    }
  }
}
