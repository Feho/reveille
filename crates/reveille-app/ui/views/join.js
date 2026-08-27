// SPDX-License-Identifier: GPL-2.0-only

// The detail pane. Selecting a server previews the join in place, so the list
// never disappears and servers stay comparable.
//
// This is where three of the four canonical state names are rendered — Needs N
// maps, No download for N maps, Map list not published — because this is where
// the decision is made. The list deliberately does not repeat them as badges.
// Each name states what Reveille measured rather than how confident it feels
// about it (lib/format.js `stateName`, docs/ux-standards.md §1.1).
//
// The fourth, Compatible, is rendered nowhere. A ready server has nothing to
// qualify, and a heading reading `Compatible` above a button reading `Join`
// restates the control beneath it. Silence is the correct rendering of "nothing
// to do" (docs/ui.md §9).
//
// The join gate is about the map running *now*, not the whole rotation. A server
// with one unobtainable map later in its rotation is perfectly playable until it
// reaches that map; refusing the join would invent a problem the engine does not
// have. What is refused is joining a server whose current map is absent, because
// that connection fails immediately.

import { el, fill, frag, preserveFocus } from "../lib/dom.js";
import {
  bytes,
  displayPath,
  launchedLabel,
  mapKey,
  mapName,
  nonResultReason,
  plural,
  stateExplanation,
  stateName,
} from "../lib/format.js";
import { historyByAddress, isFavourite, toggleFavourite } from "../lib/bookmarks.js";
import {
  GAME_LABELS,
  canRecheck,
  playableGames,
  selectedRow,
  state,
  update,
} from "../lib/store.js";

export function joinView(root, { onJoin, onRecheck }) {
  const scroll = el("div", { className: "detail-pane__scroll" });
  const actions = el("div", { className: "actions" });
  fill(root, scroll, actions);

  const render = () => {
    const row = selectedRow();
    // A selection whose row has left the list because a check found it gone. The pane keeps the
    // player's place and says what the check found, rather than emptying with no explanation.
    const gone = !row && state.selected ? state.checks.get(state.selected) : null;
    if (!row && !gone?.dropped) {
      fill(scroll, idlePlaceholder());
      fill(actions);
      actions.classList.add("hidden");
      return;
    }
    actions.classList.remove("hidden");
    preserveFocus(root, () => {
      fill(scroll, row ? body(row, onRecheck) : gonePane(state.selected, gone));
      fill(
        actions,
        ...(row ? actionBar(row, onJoin) : goneActions(state.selected, gone, onRecheck)),
      );
    });
  };

  return { render };
}

function idlePlaceholder() {
  return el(
    "div",
    { className: "placeholder" },
    el("h3", null, "No server selected"),
    el("p", null, "Pick one to see what it needs."),
  );
}

function body(row, onRecheck) {
  const { server, compatibility } = row;
  const preview = state.preview?.address === row.address ? state.preview : null;
  const assessment = preview?.assessment ?? compatibility;
  const run = state.installRun;
  const result = state.joinResult?.address === row.address ? state.joinResult : null;

  return frag(
    header(row, server, onRecheck),
    facts(server),
    result ? outcomeSection(result) : null,
    run ? installSection(run) : null,
    !run && !result ? needsSection(assessment, preview, server) : null,
  );
}

function header(row, server, onRecheck) {
  const starred = isFavourite(row.address);
  const launched = launchedLabel(historyByAddress().get(row.address));
  return el(
    "div",
    { className: "detail__head" },
    el("p", { className: "label" }, "Server"),
    el(
      "div",
      { className: "detail__title-row" },
      el("h2", { className: "detail__title" }, server.hostname || "(unnamed server)"),
      el(
        "button",
        {
          type: "button",
          className: "star star--lg",
          dataset: { focusKey: "detail-star" },
          "aria-pressed": starred ? "true" : "false",
          "aria-label": `Favourite ${server.hostname || row.address}`,
          title: starred ? "Remove from favourites" : "Add to favourites",
          onclick: () => {
            toggleFavourite(row);
            update(() => {});
          },
        },
        starred ? "★" : "☆",
      ),
    ),
    el("p", { className: "data quiet selectable" }, row.address),
    // What Reveille did, not what the server did: it started the game and saw it start. Whether
    // this server admitted the player is decided at connect time and never observed (H12).
    launched &&
      el(
        "p",
        {
          className: "quiet",
          title:
            "Reveille started the game connecting to this server. Whether the server let you in is not something Reveille can see.",
        },
        launched,
      ),
    freshness(row, onRecheck),
  );
}

/**
 * When this row was measured, and the one control that changes the answer.
 *
 * Every figure above it — the client count, the map, the round trip — was taken at one moment and
 * has been ageing since. Nothing else on screen says which moment, so a server that filled up ten
 * minutes ago still reads as empty. **Check again** asks this one server, and the time is what
 * shows it happened when the answer comes back the same.
 *
 * Hidden while a sweep runs: that is already re-asking every server in the list, this row included.
 */
function freshness(row, onRecheck) {
  if (state.browse.running) return null;
  const check = state.checks.get(row.address);
  return el(
    "div",
    { className: "detail__freshness" },
    el(
      "div",
      { className: "detail__freshness-row" },
      checkedLine(row.address),
      el(
        "button",
        {
          type: "button",
          className: "btn btn--sm",
          // `aria-disabled`, not `disabled`: this button disables itself the moment it is pressed,
          // and focus cannot be restored to a disabled element after the repaint, so a keyboard
          // player would lose the caret on every check. `canRecheck` refuses the press instead.
          "aria-disabled": canRecheck(row.address) ? null : "true",
          dataset: { focusKey: "detail-recheck" },
          title: "Ask this one server again, without re-checking the whole list.",
          onclick: () => onRecheck(row),
        },
        check?.status === "checking" ? "Checking…" : "Check again",
      ),
    ),
    // A command that never ran is not a server that did not answer, and the figures above are still
    // the last thing actually measured. Saying so is what stops an unchanged timestamp from reading
    // as a fresh confirmation.
    check?.status === "failed"
      ? el("p", { className: "error", role: "alert" }, `The check could not run. ${check.error}`)
      : null,
  );
}

/**
 * When the figures on this row were measured.
 *
 * A row the sweep returned is timed by when the sweep finished, which is not exactly when that row
 * answered — probes stream in across the whole run — so it is worded as the check it came from
 * rather than as a measurement of this one server. A row a single check re-asked has its own time,
 * and that one can say so plainly.
 */
function checkedLine(address) {
  const own = state.checkedAt.get(address);
  if (own) {
    return el(
      "p",
      { className: "quiet", title: "This server was asked again then. The figures are that reply." },
      `Checked at ${own}`,
    );
  }
  const swept = state.browse.completedAt;
  if (!swept) return el("p", { className: "quiet" }, "");
  return el(
    "p",
    { className: "quiet", title: "The figures on this row come from that list, and not since." },
    `From the server list at ${swept}`,
  );
}

/**
 * The selected server, after a check that ran and found it no longer there.
 *
 * The row has left the list, because a check that got no answer is evidence about now and the
 * client count, map and round trip it replaced are not (docs/rules.md H12). Emptying the pane
 * instead would lose the player's place and say nothing about why, so what is left is the name it
 * had, the address, what the check found, and the one thing that can change the answer.
 *
 * The name is drawn in the remembered style, like an absent row's, because it is the only thing
 * here that came from a past reading.
 */
function gonePane(address, check) {
  return frag(
    el(
      "div",
      { className: "detail__head" },
      el("p", { className: "label" }, "Server"),
      el(
        "h2",
        { className: "detail__title detail__title--remembered" },
        check.dropped.hostname || "(unnamed server)",
      ),
      el("p", { className: "data quiet selectable" }, address),
    ),
    el(
      "div",
      { className: "detail__section" },
      el("p", { className: "label" }, "Last check"),
      el("h3", { className: "display heading-sm" }, goneHeadline(check)),
      el("p", { className: "quiet" }, goneDetail(check, address)),
    ),
  );
}

function goneHeadline(check) {
  if (check.status === "checking") return "Checking…";
  if (check.status === "failed") return "The check did not run";
  if (check.otherGame) return `Runs ${GAME_LABELS[check.otherGame] ?? check.otherGame}`;
  if (check.movedTo) return "Answers at another address";
  return "Did not answer";
}

function goneDetail(check, address) {
  if (check.status === "checking") return `Asking ${address} again.`;
  if (check.status === "failed") return `The check could not run. ${check.error}`;
  if (check.otherGame) {
    const name = GAME_LABELS[check.otherGame] ?? check.otherGame;
    return playableGames(state.install).includes(check.otherGame)
      ? `It answered for ${name}. Switch this session to ${name} to join it.`
      : `It answered for ${name}, which this game folder cannot run.`;
  }
  // "Publishes", not "replied from": the reply came from the query port that was asked. What moved
  // is the game address the server publishes in it.
  if (check.movedTo) {
    return `It now publishes ${check.movedTo} as its game address, which is in the list.`;
  }
  if (check.nonResult) return `This server ${nonResultReason(check.nonResult)}.`;
  return "This server did not answer.";
}

/**
 * The facts about this server that the row above does not already carry.
 *
 * The published rotation's *size* used to sit here as a **Map list** row. It went with the
 * rotation listing itself on 27 Aug 2026: how many maps a server intends to play later is not
 * something a player decides on, and in the one case where it mattered — no list published at
 * all — the state below says so in a sentence rather than leaving the reader to infer it from
 * the words "not published" beside a heading.
 */
function facts(server) {
  return el(
    "div",
    { className: "detail__section" },
    el(
      "dl",
      { className: "kv" },
      el("dt", null, "Map"),
      el("dd", null, server.current_map ? mapName(server.current_map) : "not published"),
      server.reserved_slots
        ? el("dt", null, "Reserved")
        : null,
      server.reserved_slots
        ? el("dd", null, `${plural(server.reserved_slots, "slot")} held back`)
        : null,
    ),
  );
}

/**
 * What this server needs — and nothing at all when it needs nothing.
 *
 * Until 27 Aug 2026 this was two sections. **Before you join** restated a verdict the primary
 * button already carries in its own label, and **Maps** listed the whole published rotation,
 * every map already on disk included, under headings that were mostly empty. A ready server —
 * the ordinary case, and the one a player is trying to pick out of the list — drew two headings,
 * a state name and a paragraph of maps it already has, and pushed the address and the freshness
 * line below the fold to do it. docs/ui.md §9 had already ruled on this: *a ready server says
 * nothing; silence is the correct rendering of "nothing to do"*, and an explanation earns a
 * paragraph only if it changes the next click.
 *
 * So this returns `null` outright for a compatible server with nothing to qualify. What survives
 * is what changes the click: the state and how it was reached, what it costs, and the one choice
 * Reveille refuses to make on the player's behalf.
 *
 * The rotation is not drawn at all. A map already on disk needs no row; a missing map that
 * resolves is a number in the button, not a list to read; and *which* maps have no download
 * changes nothing the player can do about them, so the state name counts them and stops there.
 * The single map that does block a join — the one running right now — is named by the action bar,
 * which is where the block is.
 */
function needsSection(assessment, preview, server) {
  const resolving = state.previewProgress && !preview;
  const totals = preview ? shoppingTotals(preview) : null;
  const explanation = stateExplanation(assessment.state);
  const choices = (preview?.catalogue?.resolutions ?? []).filter(
    (resolution) => resolution.outcome === "choice_required",
  );
  const notes = caveats(server, assessment.state?.state === "compatible");
  const costly = Boolean(totals && totals.count > 0);

  if (!resolving && !explanation && !costly && !choices.length && !notes && !state.previewError) {
    return null;
  }

  return el(
    "div",
    { className: "detail__section" },
    // The state name is drawn only when it qualifies something. "Compatible" over a button that
    // already reads `Join` is a heading restating the control beneath it.
    explanation ? el("h3", { className: "display heading-sm" }, stateName(assessment.state)) : null,
    // Persistent, not a tooltip: this sentence is what makes the name above it a decision, and a
    // title is unreachable by keyboard and by touch (docs/ux-standards.md §3.1).
    explanation ? el("p", { className: "verdict-note" }, explanation) : null,
    resolving ? resolvingMeter() : null,
    costly
      ? el(
          "div",
          { className: "headline-number" },
          el("strong", null, bytes(totals.size)),
          el(
            "span",
            { className: "quiet" },
            `to fetch · ${plural(totals.count, "file")}${totals.pending ? ` · ${totals.pending} awaiting a choice` : ""}`,
          ),
        )
      : null,
    state.previewError ? el("p", { className: "error", role: "alert" }, state.previewError) : null,
    choices.length
      ? el(
          "div",
          { className: "rot" },
          el(
            "p",
            {
              className: "label group-title",
              title:
                "The catalogue files these under a different name. Reveille never picks for you, and checks the archive contents after downloading.",
            },
            "Needs your choice",
          ),
          choices.map((resolution) => choiceBlock(resolution)),
        )
      : null,
    notes,
  );
}

function resolvingMeter() {
  const { index, of, map } = state.previewProgress;
  const percent = of > 0 ? Math.round(((index + 1) / of) * 100) : 0;
  return el(
    "div",
    { className: "stack--tight" },
    el(
      "div",
      {
        className: "meter",
        role: "progressbar",
        "aria-label": "Looking up missing maps",
        "aria-valuenow": index + 1,
        "aria-valuemin": 0,
        "aria-valuemax": of,
      },
      el("span", { className: "meter__fill", style: `width:${percent}%` }),
    ),
    el("p", { className: "quiet data" }, `looking up ${mapName(map)} · ${index + 1}/${of}`),
  );
}

function line(mark, kind, name, trailing) {
  return el(
    "div",
    { className: "rot__row" },
    el("span", { className: `rot__mark rot__mark--${kind}`, "aria-hidden": "true" }, mark),
    el("span", { className: "rot__name", title: name }, name),
    trailing ? el("span", { className: "rot__size" }, trailing) : null,
  );
}

function choiceBlock(resolution) {
  const name = resolution.wanted.name;
  const chosen = state.choices.get(name) ?? null;
  return frag(
    line("?", "choose", mapName(name), plural(resolution.choices.length, "candidate")),
    el(
      "div",
      { className: "choices", role: "radiogroup", "aria-label": `Source for ${mapName(name)}` },
      resolution.choices.map((candidate) =>
        el(
          "label",
          { className: "choice" },
          el("input", {
            type: "radio",
            name: `choice-${name}`,
            value: String(candidate.id),
            checked: chosen === candidate.id,
            dataset: { focusKey: `choice-${name}-${candidate.id}` },
            onchange: () => update((next) => next.choices.set(name, candidate.id)),
          }),
          el(
            "span",
            { className: "choice__body" },
            el("span", { className: "choice__file" }, candidate.filename),
            el(
              "span",
              { className: "choice__meta" },
              `${bytes(candidate.file_size)} · ${candidate.downloads} downloads${candidate.map_file_tested ? " · tested" : ""}`,
            ),
          ),
        ),
      ),
    ),
  );
}

function installSection(run) {
  return el(
    "div",
    { className: "detail__section" },
    el("p", { className: "label" }, run.done ? "Finished" : "Getting files"),
    el(
      "div",
      { className: "install-list" },
      [...run.items.values()].map((item) => installItem(item)),
    ),
    run.items.size === 0 ? el("p", { className: "quiet" }, "Preparing…") : null,
  );
}

function installItem(item) {
  const percent =
    item.total && item.total > 0 ? Math.min(100, Math.round((item.received / item.total) * 100)) : 0;
  let stateText = "waiting";
  if (item.phase === "downloading") stateText = `${bytes(item.received)} / ${bytes(item.total)}`;
  else if (item.phase === "confirming") stateText = "checking the file";
  else if (item.phase === "installed") stateText = "installed";
  else if (item.phase === "failed") stateText = "failed";

  return el(
    "div",
    { className: "install-item" },
    el(
      "div",
      { className: "install-item__head" },
      el("span", { className: "install-item__name", title: item.filename }, mapName(item.map)),
      el(
        "span",
        { className: `install-item__state${item.phase === "failed" ? " failed" : ""}` },
        stateText,
      ),
    ),
    item.phase === "downloading"
      ? el(
          "div",
          { className: "meter" },
          el("span", { className: "meter__fill", style: `width:${percent}%` }),
        )
      : null,
    item.phase === "failed" ? el("p", { className: "quiet" }, item.reason) : null,
  );
}

function outcomeSection(result) {
  const launched = result.outcome.launch === "launched";
  return el(
    "div",
    { className: "detail__section" },
    el("p", { className: "label" }, launched ? "Launched" : "Not launched"),
    el(
      "h3",
      { className: "display heading-sm" },
      launched ? "The game is starting" : stateName(result.assessment.state),
    ),
    el(
      "p",
      { className: "quiet" },
      launched
        ? `${GAME_LABELS[result.game] ?? "The game"} is connecting. The server decides the rest: bans, a full server, and its own ping limits.`
        : result.outcome.reason,
    ),
    result.installed.length
      ? el(
          "p",
          { className: "quiet" },
          `${plural(result.installed.length, "file")} installed into `,
          el("span", { className: "data selectable" }, displayPath(result.game_directory)),
        )
      : null,
    result.used_home_fallback
      ? el(
          "p",
          { className: "note note--brass" },
          el("strong", null, "The install folder is not writable. "),
          "Maps went to ",
          el("span", { className: "data selectable" }, displayPath(result.game_directory)),
          ", which the engine searches first.",
        )
      : null,
    result.failures.length
      ? el(
          "div",
          null,
          el("p", { className: "label group-title" }, "Not installed"),
          el(
            "ul",
            { className: "rot" },
            result.failures.map((failure) =>
              el(
                "li",
                { className: "rot__row" },
                el("span", { className: "rot__name" }, mapName(failure.map)),
                el("span", { className: "rot__size" }, failure.reason),
              ),
            ),
          ),
        )
      : null,
  );
}

/**
 * The limits of the check just above, stated where the check is read.
 *
 * These are not general trivia about the server: each one is a reason the verdict may be weaker
 * than it looks, so they sit inside the verdict section rather than under a heading of their own.
 * A missing checksum in particular means "compatible" was decided on names alone, and saying so
 * is what keeps that word honest.
 */
/**
 * The two things about this server that qualify what Reveille checked.
 *
 * Each is drawn only where it changes something (docs/ui.md §9).
 *
 * **Sends no files** is about maps that are missing, so it is silent when none are: telling a
 * player that a server they can already join will not send them anything is a sentence about a
 * situation they are not in.
 *
 * **No map checksum** is drawn always, including on a server with nothing to fetch, because it
 * qualifies the reading itself. Every "on disk" in this pane was decided by name alone, and that
 * is the one caveat a ready server does not get to keep quiet about.
 */
function caveats(server, ready) {
  const notes = [];
  if (server.allow_download === 0 && !ready) {
    notes.push("Sends no files — anything missing has to be here before you join.");
  }
  if (server.map_checksum === null || server.map_checksum === undefined) {
    notes.push("Publishes no map checksum, so only names are matched, not files.");
  }
  if (!notes.length) return null;
  return el(
    "div",
    { className: "stack--tight" },
    notes.map((note) => el("p", { className: "quiet" }, note)),
  );
}

/** What the current selection would cost, counting only what will actually be fetched. */
export function shoppingTotals(preview) {
  let size = 0;
  let count = 0;
  let pending = 0;
  for (const resolution of preview?.catalogue?.resolutions ?? []) {
    if (resolution.outcome === "exact") {
      size += Number(resolution.name_match.file_size);
      count += 1;
    } else if (resolution.outcome === "choice_required") {
      const chosen = state.choices.get(resolution.wanted.name);
      const candidate = resolution.choices.find((item) => item.id === chosen);
      if (candidate) {
        size += Number(candidate.file_size);
        count += 1;
      } else {
        pending += 1;
      }
    }
  }
  return { size, count, pending };
}

/**
 * The one control a gone server offers: ask it again.
 *
 * Offered whatever the check found, including a server that answered for another game — unlike
 * an absent bookmark, this row *was* in this game's list a moment ago, so what the check found is
 * a change and asking again is the way to see whether it changed back.
 */
function goneActions(address, check, onRecheck) {
  return [
    el(
      "button",
      {
        type: "button",
        className: "btn btn--block",
        // Focusable while busy, for the same reason as the freshness control above.
        "aria-disabled": canRecheck(address) ? null : "true",
        dataset: { focusKey: "detail-recheck" },
        onclick: () =>
          onRecheck({ address, server: { endpoint: { query_port: check.dropped.queryPort } } }),
      },
      check.status === "checking" ? "Checking…" : "Check again",
    ),
  ];
}

function actionBar(row, onJoin) {
  const preview = state.preview?.address === row.address ? state.preview : null;
  const assessment = preview?.assessment ?? row.compatibility;
  const kind = assessment.state.state;
  const readiness = assessment.current_map?.readiness ?? "unknown";
  const busy = Boolean(state.joining);
  // A probe in flight can drop this row before the join command returns, and the outcome would then
  // have no row to render against. One request either way; waiting for it costs a moment.
  const checking = state.checks.get(row.address)?.status === "checking";
  const resolving = Boolean(state.previewProgress && !preview);
  const totals = preview ? shoppingTotals(preview) : { size: 0, count: 0, pending: 0 };
  const result = state.joinResult?.address === row.address ? state.joinResult : null;

  if (result?.outcome.launch === "launched") {
    return [
      el(
        "button",
        { type: "button", className: "btn btn--block", onclick: () => update((next) => (next.joinResult = null)) },
        "Back to server details",
      ),
    ];
  }

  // The map running right now being absent is the one thing consent cannot buy —
  // that connection is dropped on arrival. But it only blocks the join if the map
  // cannot be fetched. When it is in the shopping list, downloading is precisely
  // the fix, and refusing to let the player start the download would strand them
  // on the one screen that could have solved it.
  const currentMap = row.server.current_map;
  const fetchable = currentMapFetchable(preview, currentMap);
  if (readiness === "missing" && kind !== "compatible" && fetchable === "no") {
    return [
      el(
        "p",
        { className: "note note--bad" },
        `${mapName(currentMap)} is running now, is not on disk, and is not in the catalogue. Joining would drop you immediately.`,
      ),
      el(
        "button",
        { type: "button", className: "btn btn--block", disabled: true },
        "Cannot join while this map is running",
      ),
    ];
  }

  const rows = [];
  if (readiness === "missing" && fetchable === "yes") {
    rows.push(
      el(
        "p",
        { className: "note note--brass" },
        `${mapName(currentMap)} is running now and is not on disk. Fetching is what makes this join work.`,
      ),
    );
  } else if (readiness === "missing" && fetchable === "choose") {
    rows.push(
      el(
        "p",
        { className: "note note--brass" },
        `${mapName(currentMap)} is running now and is not on disk. Pick a source for it above.`,
      ),
    );
  }
  if (totals.pending > 0) {
    rows.push(
      el(
        "p",
        { className: "quiet" },
        `${totals.pending} ${totals.pending === 1 ? "map needs" : "maps need"} a choice above.`,
      ),
    );
  }
  rows.push(
    el(
      "div",
      { className: "actions__row" },
      el(
        "button",
        {
          type: "button",
          className: "btn btn--primary",
          disabled: busy || resolving || checking,
          dataset: { focusKey: "join" },
          // Consent is the click. The label names what this join is missing, so a
          // separate confirmation toggle would only add a step to the same answer.
          onclick: () => onJoin(row, kind !== "compatible"),
        },
        busy ? "Working…" : checking ? "Checking…" : joinLabel(kind, totals),
      ),
    ),
  );
  if (state.joinError) {
    rows.push(el("p", { className: "error", role: "alert" }, state.joinError));
  }
  return rows;
}

/** The primary button's label, which is also the consent it records. */
function joinLabel(kind, totals) {
  if (totals.count > 0) return `Get ${bytes(totals.size)} & join`;
  if (kind === "compatible") return "Join";
  if (kind === "cant_tell") return "Join without a map list";
  return "Join anyway";
}

/**
 * Can the map the server is running right now be fetched?
 *
 * "yes" — it resolved to an exact catalogue match, or the player already chose a
 * source for it. "choose" — candidates exist but none is selected yet.
 * "no" — the catalogue has nothing. "unknown" — resolution has not run or does not
 * cover it, in which case nothing is claimed and the backend decides after the
 * rescan.
 */
function currentMapFetchable(preview, currentMap) {
  const key = mapKey(currentMap);
  if (!preview || key === null) return "unknown";
  const resolution = (preview.catalogue?.resolutions ?? []).find(
    (item) => mapKey(item.wanted.name) === key,
  );
  if (!resolution) return "unknown";
  if (resolution.outcome === "exact") return "yes";
  if (resolution.outcome === "no_source") return "no";
  return state.choices.has(resolution.wanted.name) ? "yes" : "choose";
}
