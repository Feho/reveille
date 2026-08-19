// SPDX-License-Identifier: GPL-2.0-only

// The detail pane. Selecting a server previews the join in place, so the list
// never disappears and servers stay comparable.
//
// This is where the four canonical state names live — Compatible, Needs N maps,
// No source, Can't tell — because this is where the decision is made. The list
// deliberately does not repeat them as badges.
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
  engineLabel,
  mapKey,
  mapName,
  plural,
  stateExplanation,
  stateName,
} from "../lib/format.js";
import { selectedRow, state, update } from "../lib/store.js";

export function joinView(root, { onJoin }) {
  const scroll = el("div", { className: "detail-pane__scroll" });
  const actions = el("div", { className: "actions" });
  fill(root, scroll, actions);

  const render = () => {
    const row = selectedRow();
    if (!row) {
      fill(scroll, idlePlaceholder());
      fill(actions);
      actions.classList.add("hidden");
      return;
    }
    actions.classList.remove("hidden");
    preserveFocus(root, () => {
      fill(scroll, body(row));
      fill(actions, ...actionBar(row, onJoin));
    });
  };

  return { render };
}

function idlePlaceholder() {
  return el(
    "div",
    { className: "placeholder" },
    el("h3", null, "No server selected"),
    el(
      "p",
      null,
      "Pick a server to see what it is running, what its rotation needs, and exactly what Reveille would fetch before you join.",
    ),
  );
}

function body(row) {
  const { server, compatibility } = row;
  const preview = state.preview?.address === row.address ? state.preview : null;
  const assessment = preview?.assessment ?? compatibility;
  const run = state.installRun;
  const result = state.joinResult?.address === row.address ? state.joinResult : null;

  return frag(
    header(row, server),
    facts(server),
    result ? outcomeSection(result) : null,
    run ? installSection(run) : null,
    !run && !result ? verdictSection(assessment, preview) : null,
    !run && !result ? rotationSection(row, assessment, preview) : null,
    warnings(server),
  );
}

function header(row, server) {
  return el(
    "div",
    { className: "detail__head" },
    el("p", { className: "label" }, "Server"),
    el("h2", { className: "detail__title" }, server.hostname || "(unnamed server)"),
    el("p", { className: "data quiet selectable" }, row.address),
  );
}

function facts(server) {
  const rotation = server.rotation?.length ?? 0;
  return el(
    "div",
    { className: "detail__section" },
    el(
      "dl",
      { className: "kv" },
      el("dt", null, "Engine"),
      el("dd", null, engineLabel(server)),
      el("dt", null, "Now"),
      el("dd", null, server.current_map ? mapName(server.current_map) : "not published"),
      el("dt", null, "Rotation"),
      el("dd", null, rotation ? plural(rotation, "map") : "not published"),
      server.reserved_slots
        ? el("dt", null, "Reserved")
        : null,
      server.reserved_slots
        ? el("dd", null, `${plural(server.reserved_slots, "slot")} held back`)
        : null,
      server.join_window
        ? el("dt", null, "Join window")
        : null,
      server.join_window
        ? el("dd", null, `closes ${server.join_window}s after a round starts`)
        : null,
    ),
  );
}

function verdictSection(assessment, preview) {
  const resolving = state.previewProgress && !preview;
  const totals = preview ? shoppingTotals(preview) : null;

  return el(
    "div",
    { className: "detail__section" },
    el("p", { className: "label" }, "Join check"),
    el("h3", { className: "display heading-sm" }, stateName(assessment.state)),
    el("p", { className: "quiet" }, stateExplanation(assessment.state)),
    resolving ? resolvingMeter() : null,
    totals && totals.count > 0
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

/** Group the rotation the way a decision needs it, not the way the wire sent it. */
function rotationSection(row, assessment, preview) {
  const maps = assessment.preflight?.maps ?? [];
  if (!maps.length) {
    return el(
      "div",
      { className: "detail__section" },
      el("p", { className: "label" }, "Rotation"),
      el(
        "p",
        { className: "quiet" },
        "This server publishes no map list. Reveille shows silence as silence rather than turning it into a green tick.",
      ),
    );
  }

  const present = maps.filter((entry) => entry.status.status === "present");
  const wanted = maps.filter((entry) => entry.status.status !== "present");
  const resolutions = new Map(
    (preview?.catalogue?.resolutions ?? []).map((item) => [item.wanted.name, item]),
  );

  const exact = [];
  const choose = [];
  const none = [];
  const unresolved = [];
  for (const entry of wanted) {
    const resolution = resolutions.get(entry.map);
    if (!resolution) unresolved.push(entry);
    else if (resolution.outcome === "exact") exact.push(resolution);
    else if (resolution.outcome === "choice_required") choose.push(resolution);
    else none.push(resolution);
  }

  return el(
    "div",
    { className: "detail__section" },
    el("p", { className: "label" }, "Rotation"),
    el(
      "div",
      { className: "rot" },
      present.length
        ? group(
            `On disk — nothing to do`,
            el(
              "p",
              { className: "quiet indent" },
              `${plural(present.length, "map")}: ${present
                .slice(0, 4)
                .map((entry) => mapName(entry.map))
                .join(", ")}${present.length > 4 ? `, and ${present.length - 4} more` : ""}`,
            ),
          )
        : null,
      exact.length
        ? group(
            "Matched in the catalogue",
            exact.map((resolution) =>
              frag(
                line("↓", "get", mapName(resolution.wanted.name), bytes(resolution.name_match.file_size)),
                el("p", { className: "rot__file" }, resolution.name_match.filename),
              ),
            ),
          )
        : null,
      choose.length
        ? group(
            "Needs your choice",
            choose.map((resolution) => choiceBlock(resolution)),
          )
        : null,
      none.length
        ? group(
            "No source",
            none.map((resolution) =>
              frag(
                line("✕", "none", mapName(resolution.wanted.name), "—"),
                el(
                  "p",
                  { className: "rot__file" },
                  "Not in the catalogue. You can still play here; you will be dropped when the rotation reaches this map.",
                ),
              ),
            ),
          )
        : null,
      unresolved.length
        ? group(
            "Missing locally",
            unresolved.map((entry) =>
              line(
                entry.status.status === "absent" ? "•" : "≠",
                "choose",
                mapName(entry.map),
                entry.status.status === "absent" ? "not on disk" : "different file",
              ),
            ),
          )
        : null,
    ),
  );
}

function group(title, ...children) {
  return el(
    "div",
    { className: "rot__group" },
    el("p", { className: "label group-title" }, title),
    children,
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
      el(
        "p",
        { className: "quiet" },
        "The catalogue files this map under a different name, so Reveille will not pick for you. It checks the archive contents after downloading and tells you if the choice was wrong.",
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
  else if (item.phase === "confirming") stateText = "checking archive";
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
        ? "Allied Assault is connecting now. Bans, a full server and ping limits are decided by the server at this point — Reveille cannot check those in advance."
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

/** Facts about this server that change how a join goes, stated once, without alarm. */
function warnings(server) {
  const notes = [];
  if (server.allow_download === 0) {
    notes.push("This server will not send you files. Anything missing has to be here before you join.");
  }
  if (server.map_checksum === null || server.map_checksum === undefined) {
    notes.push(
      "This server publishes no map checksum, so an exact-file match cannot be confirmed in advance — only the map name.",
    );
  }
  if (!notes.length) return null;
  return el(
    "div",
    { className: "detail__section" },
    el("p", { className: "label" }, "Worth knowing"),
    notes.map((note) => el("p", { className: "quiet" }, note)),
    el(
      "p",
      { className: "note" },
      el("strong", null, "Bans, kicks and a full server "),
      "are decided when you actually connect. Nothing queried beforehand can rule them out, so Reveille does not imply it has.",
    ),
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

function actionBar(row, onJoin) {
  const preview = state.preview?.address === row.address ? state.preview : null;
  const assessment = preview?.assessment ?? row.compatibility;
  const kind = assessment.state.state;
  const readiness = assessment.current_map?.readiness ?? "unknown";
  const busy = Boolean(state.installRun && !state.installRun.done);
  const resolving = Boolean(state.previewProgress && !preview);
  const totals = preview ? shoppingTotals(preview) : { size: 0, count: 0, pending: 0 };
  const result = state.joinResult?.address === row.address ? state.joinResult : null;

  if (result?.outcome.launch === "launched") {
    return [
      el(
        "button",
        { type: "button", className: "btn btn--block", onclick: () => update((next) => (next.joinResult = null)) },
        "Back to the check",
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
        `This server is running ${mapName(currentMap)}, which is not on disk and is not in the catalogue. Joining now would drop you immediately.`,
      ),
      el(
        "button",
        { type: "button", className: "btn btn--block", disabled: true },
        "Cannot join yet",
      ),
    ];
  }

  const needsConsent = kind !== "compatible";
  const blocked = needsConsent && !state.acceptIncomplete;
  const rows = [];

  if (readiness === "missing" && fetchable === "yes") {
    rows.push(
      el(
        "p",
        { className: "note note--brass" },
        el("strong", null, `${mapName(currentMap)} is running now and is not on disk. `),
        "It is in the files below, so fetching them is what makes this join work.",
      ),
    );
  } else if (readiness === "missing" && fetchable === "choose") {
    rows.push(
      el(
        "p",
        { className: "note note--brass" },
        el("strong", null, `${mapName(currentMap)} is running now and is not on disk. `),
        "Pick a source for it above and Reveille will fetch it with the rest.",
      ),
    );
  }
  if (needsConsent) {
    rows.push(consentToggle(kind, totals));
  }
  if (totals.pending > 0) {
    rows.push(
      el(
        "p",
        { className: "quiet" },
        `${totals.pending} ${totals.pending === 1 ? "map still needs" : "maps still need"} a choice above. Reveille never picks an ambiguous match for you.`,
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
          disabled: busy || resolving || blocked,
          dataset: { focusKey: "join" },
          onclick: () => onJoin(row),
        },
        busy ? "Working…" : totals.count > 0 ? `Get ${bytes(totals.size)} & join` : "Join",
      ),
    ),
  );
  if (state.joinError) {
    rows.push(el("p", { className: "error", role: "alert" }, state.joinError));
  }
  return rows;
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

function consentToggle(kind, totals) {
  const label =
    kind === "cant_tell"
      ? "Join without a rotation check"
      : kind === "no_source"
        ? "Join anyway, knowing a map is missing"
        : totals.count > 0
          ? "Join after fetching what is missing"
          : "Join without everything the rotation needs";
  return el(
    "button",
    {
      type: "button",
      className: "toggle",
      "aria-pressed": state.acceptIncomplete ? "true" : "false",
      dataset: { focusKey: "accept-incomplete" },
      onclick: () => update((next) => (next.acceptIncomplete = !next.acceptIncomplete)),
    },
    el("span", { className: "toggle__box", "aria-hidden": "true" }, "✓"),
    label,
  );
}
