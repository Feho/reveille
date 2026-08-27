// SPDX-License-Identifier: GPL-2.0-only

// The closed lexicon, in one place the player can reach.
//
// Reveille runs a small vocabulary of terms of art, and each one draws a line the rest of the
// interface depends on: *listed* is not *replied*, *launched* is not *joined*, *clients* is not
// *players*. Those distinctions are the product. Left undefined they read as pedantry, and the
// rigour leaks as confusion.
//
// One definition per term, stated once, reachable identically from setup and from the shell —
// which is also what WCAG 2.2 SC 3.2.6 Consistent Help asks for (docs/ux-standards.md §6.2).
//
// Each entry obeys the string rule in docs/ux-standards.md §2.8: at most 25 words, at most two
// sentences, active voice, no negative contraction, and any hedge stated as a measurement rather
// than as a mood word.

import { el } from "./dom.js";
import { openDialog } from "./dialog.js";

const TERMS = [
  [
    "Master list",
    "The directory that names which servers exist. Reveille asks it first, then asks each server in turn.",
  ],
  [
    "Listed",
    "The master list named this server. Reveille has not asked the server itself yet.",
  ],
  [
    "Replied",
    "Reveille asked this server and it answered. Every figure on the row comes from that one reply.",
  ],
  [
    "Did not answer",
    "Reveille asked this server and heard nothing back before the timeout. The server may still be running.",
  ],
  [
    "Not in this list",
    "This server was missing from the master list, so Reveille never asked it. It may still be running.",
  ],
  [
    "Players",
    "Occupied slots, as the server reports them. Bots are not in this figure. A slot still counts while its player is connecting or downloading, so this is a count of connections rather than of people at play.",
  ],
  [
    "Bots",
    "Counted separately by the server, and never added to the player figure. The two are different quantities, and a server that publishes one may not publish the other.",
  ],
  [
    "Ping",
    "Time for one status request to this server and back, measured once. This is not the in-game ping.",
  ],
  [
    "Map list",
    "The maps a server publishes that it will move through as play continues. Some servers publish none.",
  ],
  [
    "Needs 3 maps",
    "This server's map list includes 3 maps you do not have. Reveille can download them before you join.",
  ],
  [
    "No download for 3 maps",
    "Those maps are in no catalogue Reveille can reach. You are dropped when the map list reaches them.",
  ],
  [
    "Map list not published",
    "This server published no map list. Reveille checked only the map the server is running now.",
  ],
  [
    "Launched",
    "Reveille started the game connecting to this server. Whether the server let you in is not something Reveille can see.",
  ],
  [
    "Not empty",
    "Shows only servers reporting at least one occupied slot. A server running bots alone is empty by this count.",
  ],
];

/** Open the lexicon. Identical from setup and from the server list. */
export function openGlossary() {
  openDialog(
    "What these words mean",
    el(
      "p",
      { className: "quiet" },
      "Reveille says only what it measured. These words each draw a line it will not cross.",
    ),
    el(
      "dl",
      { className: "kv kv--stacked" },
      TERMS.flatMap(([term, meaning]) => [
        el("dt", null, term),
        el("dd", null, meaning),
      ]),
    ),
  );
}

/** The button that opens it. Placed last in each view's own chrome. */
export function glossaryButton(className = "btn btn--sm btn--ghost") {
  return el(
    "button",
    {
      type: "button",
      className,
      onclick: (event) => {
        event.preventDefault();
        openGlossary();
      },
    },
    "What these words mean",
  );
}
