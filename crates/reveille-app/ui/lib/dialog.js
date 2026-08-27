// SPDX-License-Identifier: GPL-2.0-only

// The one modal, borrowed by whoever needs it.
//
// There is a single `<dialog>` in the document and callers set its heading and body, rather than
// each panel shipping its own markup. Focus, Esc and the backdrop then behave identically wherever
// a panel is opened from, which is the part a second hand-rolled dialog would get wrong.

import { $, fill } from "./dom.js";

/** Set the heading and body of the shared dialog, then show it modally. */
export function openDialog(title, ...nodes) {
  const dialog = $("#info-dialog");
  $("#info-dialog-title").textContent = title;
  fill($("#info-dialog-body"), ...nodes);
  if (!dialog.open) dialog.showModal();
}

/** Close it, whoever opened it. */
export function closeDialog() {
  const dialog = $("#info-dialog");
  if (dialog.open) dialog.close();
}
