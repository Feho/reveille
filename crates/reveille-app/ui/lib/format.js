// SPDX-License-Identifier: GPL-2.0-only

// Every string the player reads that is derived from pipeline data is built here.
//
// Two of these rules are product contracts, not preferences (AGENTS.md,
// docs/engine-facts.md §5):
//
//   * A client count is never called "players" or "humans". It is the number of
//     occupied slots and cannot distinguish a person from a bot or a parked
//     connection.
//   * Bots are a disjoint quantity. They are shown additively and never folded
//     into the client count, and free slots are never implied, because
//     capacity - clients is not observable.

/** Bytes as a short human size. Sub-MB values keep a decimal so 0.4 MB is not "0 MB". */
export function bytes(value) {
  if (value === null || value === undefined) return "—";
  const size = Number(value);
  if (!Number.isFinite(size)) return "—";
  if (size < 1024) return `${size} B`;
  const kb = size / 1024;
  if (kb < 1000) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 100 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

/** "1 map" / "3 maps". */
export function plural(count, singular, pluralForm = `${singular}s`) {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}

/** Strip the Windows extended-length prefix so paths read the way people write them. */
export function displayPath(value) {
  return String(value ?? "").replace(/^\\\\\?\\/, "");
}

/** The occupancy figures, kept separate. Returns nulls rather than guessing zero. */
export function occupancy(server) {
  const clients = server.occupancy?.clients_reported ?? null;
  const bots = server.occupancy?.bots_reported ?? null;
  return {
    clients,
    bots: bots && bots > 0 ? bots : null,
    capacity: server.client_capacity ?? null,
  };
}

/**
 * The full build string, for the detail pane where it has room to wrap.
 * Servers report things like "Medal of Honor Allied Assault 1.11 win-x86 Mar 5 2002".
 */
export function engineLabel(server) {
  return server.version || server.game_version || fallbackVersion(server);
}

/**
 * The tabular version, for the list. `gamever` is already short and comparable —
 * "1.11", "1.12+0.83.0" — whereas `version` is a sentence and truncates to
 * "Medal of Honor Allied" in every row, which distinguishes nothing.
 */
export function shortVersion(server) {
  return server.game_version || fallbackVersion(server);
}

function fallbackVersion(server) {
  return server.protocol ? `protocol ${server.protocol}` : "—";
}

/**
 * The engine's map-name normalisation, reproduced exactly.
 *
 * `MapKey::new` in crates/reveille-core/src/mapindex.rs, and docs/engine-facts.md §2:
 * trim, backslashes to slashes, ASCII lowercase, strip a leading `maps/` and a
 * trailing `.bsp`, and **nothing else**. Both prefixed and bare names are
 * legitimate, so no prefix may be inserted.
 *
 * Used to line a server's `mapname` up with its rotation entry, which the server
 * may have spelled differently.
 */
export function mapKey(value) {
  const normalised = String(value ?? "")
    .trim()
    .replaceAll("\\", "/")
    .toLowerCase();
  const withoutPrefix = normalised.startsWith("maps/") ? normalised.slice(5) : normalised;
  const key = withoutPrefix.endsWith(".bsp") ? withoutPrefix.slice(0, -4) : withoutPrefix;
  return key === "" ? null : key;
}

/** A rotation map name as the server spelled it, with an empty name made visible. */
export function mapName(value) {
  const name = String(value ?? "").trim();
  return name === "" ? "(unnamed)" : name;
}

/**
 * What the Needs column says for one server.
 *
 * This is the cost-not-verdict rule in one function. A ready server contributes
 * nothing to the column; extra content is priced, not flagged; an unpublished
 * rotation is reported as a fact about the server. Only the state a download
 * cannot fix is coloured.
 */
export function needsCell(state) {
  switch (state?.state) {
    case "compatible":
      return null;
    case "needs_maps":
      return {
        text: `+ ${plural(state.count, "map")}`,
        kind: "cost",
        title: `This server's rotation uses ${plural(state.count, "map")} you do not have. Reveille can fetch ${state.count === 1 ? "it" : "them"} before you join.`,
      };
    case "no_source":
      return {
        text: `${plural(state.count, "map")} unavailable`,
        kind: "blocked",
        title: `${plural(state.count, "map")} in this rotation ${state.count === 1 ? "is" : "are"} not in any catalogue Reveille can reach. You can still play until the rotation reaches ${state.count === 1 ? "it" : "them"}.`,
      };
    default:
      return {
        text: "not published",
        kind: "unknown",
        title:
          "This server does not publish its map rotation, so there is nothing to check in advance.",
      };
  }
}

/** The canonical four state names, used wherever the decision is actually made. */
export function stateName(state) {
  switch (state?.state) {
    case "compatible":
      return "Compatible";
    case "needs_maps":
      return `Needs ${plural(state.count, "map")}`;
    case "no_source":
      return "No source";
    default:
      return "Can't tell";
  }
}

/** How each state was arrived at, in the vocabulary the protocol can support. */
export function stateExplanation(state) {
  switch (state?.state) {
    case "compatible":
      return "Every map this server published is on disk. The server still decides whether you get in.";
    case "needs_maps":
      return "This server's rotation uses maps you do not have. Reveille can fetch them before you join.";
    case "no_source":
      return "At least one map in the rotation is not in any catalogue Reveille can reach.";
    default:
      return "This server published no map list, so there is nothing to check in advance.";
  }
}

/**
 * Non-result reasons in plain language.
 *
 * The stage matters as much as the reason: a timeout answering the master's
 * server-list query and a timeout answering the game query are different
 * failures, and labelling both "did not answer" makes one group look like a
 * duplicate of the other.
 */
export function nonResultReason(group) {
  if (group.reason === "duplicate_endpoint") return "is the same server registered twice";
  if (group.reason === "missing_host_port") return "did not publish a game port";

  const what =
    group.stage === "get_status" ? "the game query" : "the server-list query";
  switch (group.reason) {
    case "timeout":
      return `did not answer ${what}`;
    case "network":
      return `was unreachable for ${what}`;
    case "malformed":
      return `answered ${what} with a reply Reveille could not read`;
    default:
      return `${group.reason} at ${what}`;
  }
}
