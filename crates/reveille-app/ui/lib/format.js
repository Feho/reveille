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
 * The Ping column: the round trip of this sweep's one status request.
 *
 * Deliberately not called the in-game ping. It is a single UDP sample taken
 * while fifteen other probes were in flight, so it says "this server is roughly
 * this far away", not "you will play at this latency". The tooltip carries that
 * distinction; the column header cannot.
 *
 * The server's own `sv_minPing`/`sv_maxPing` gate is a different number and is
 * never rendered here.
 */
export function roundTrip(server) {
  const value = server.status_round_trip;
  if (value === null || value === undefined) return { text: "—", title: null };
  const millis = Number(value);
  if (!Number.isFinite(millis)) return { text: "—", title: null };
  return {
    text: `${millis} ms`,
    title:
      "Time for one status request to this server and back, measured once during this check. Not the in-game ping.",
  };
}

/**
 * The Mode column: the gametype the server publishes, as it spelled it.
 *
 * `g_gametypestring` is an ordinary server cvar. The stock engine sets it to one of seven
 * labels, but a mod may put anything there, so the value is shown verbatim rather than mapped
 * onto a fixed set or shortened to FFA/OBJ/TDM: an abbreviation Reveille invented would be a
 * claim about a server it cannot check, and an unrecognised mode would have nowhere to go.
 * A server that publishes none says so with the same em dash every other unpublished figure uses.
 */
export function gameType(server) {
  const name = String(server.game_type ?? "").trim();
  if (name === "") {
    return { text: "—", title: "This server did not publish a gametype." };
  }
  return { text: name, title: name };
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
 * The canonical four state names.
 *
 * Every one of them is a measurement, not a mood word. A verbal hedge — "can't tell", "no
 * source", "possibly compatible" — costs a reader trust in the figure and in the source, where
 * the same fact stated as a measurement costs almost none (van der Bles et al., PNAS 2020;
 * docs/ux-standards.md §1.1). So the name says what Reveille found, and the player is left to
 * draw the verdict.
 *
 * "Map list", never "rotation" — the server publishes a list, and calling it a rotation claims an
 * order that is the server's to change.
 */
export function stateName(state) {
  switch (state?.state) {
    case "compatible":
      return "Compatible";
    case "needs_maps":
      return `Needs ${plural(state.count, "map")}`;
    case "no_source":
      return `No download for ${plural(state.count, "map")}`;
    default:
      return "Map list not published";
  }
}

/**
 * How each state was arrived at, rendered as persistent text beside the name.
 *
 * Not a tooltip. This is the sentence that turns a two-word noun into a decision, and a `title`
 * is unreachable by keyboard and by touch and fails WCAG 2.2 SC 1.4.13 outright
 * (docs/ux-standards.md §3.1).
 *
 * `Compatible` returns null on purpose. A ready server says nothing: silence is the correct
 * rendering of "nothing to do" (docs/ui.md §9).
 */
export function stateExplanation(state) {
  switch (state?.state) {
    case "compatible":
      return null;
    case "needs_maps":
      return "This server's map list includes maps you do not have. Reveille can download them before you join.";
    case "no_source":
      return state.count === 1
        ? "This map is in no catalogue Reveille can reach. You can play until the map list reaches it, then you are dropped."
        : "These maps are in no catalogue Reveille can reach. You can play until the map list reaches them, then you are dropped.";
    default:
      return "This server published no map list. Reveille checked only the map it is running now.";
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
/**
 * The wall clock, to the minute — how every "when was this measured" is written.
 *
 * Absolute, not relative. These labels are drawn once and are not redrawn on a timer, so a
 * "just now" left on screen goes quietly wrong as the minutes pass, which is the one thing a
 * freshness label may not do. A clock time stays true however long it sits there.
 */
export function clockTime(date = new Date()) {
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * How long ago something happened, for the history line.
 *
 * Recent values are relative because that is how a player thinks about "did I play there
 * today"; anything older than a week becomes a date, because "43 days ago" is arithmetic the
 * reader has to undo. Returns null for a missing or unreadable timestamp rather than inventing
 * one.
 */
export function timeAgo(iso) {
  if (!iso) return null;
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return null;
  const seconds = Math.max(0, (Date.now() - then.getTime()) / 1000);
  if (seconds < 90) return "just now";
  const minutes = seconds / 60;
  if (minutes < 60) return `${Math.round(minutes)} min ago`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.round(hours)}h ago`;
  const days = hours / 24;
  if (days < 7) return `${Math.round(days)}d ago`;
  return then.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/**
 * The launch line: what Reveille did, not what the server did.
 *
 * "Launched", never "joined" or "played". Reveille starts the game process and sees that it
 * started. Whether the server admitted the player is decided at connect time and Reveille never
 * observes the answer (docs/rules.md H12).
 */
export function launchedLabel(entry) {
  if (!entry?.launches) return null;
  const when = timeAgo(entry.lastLaunchedAt);
  const times = entry.launches > 1 ? ` · ${entry.launches}×` : "";
  return when ? `Launched ${when}${times}` : `Launched ${entry.launches}×`;
}

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

/**
 * Why the server list could not be built, in the player's terms.
 *
 * The `kind` is decided in Rust, beside the errors it names, exactly as `OpenMohaaFailureKind`
 * already is — the shell never reads a cause out of a formatted message. Each one carries a cause
 * and a remedy, because the two moments this fires are where a non-technical player decides
 * whether the tool is broken or their PC is (docs/design-review.md F6).
 *
 * The original message is kept as `detail` and shown as detail, not as the whole status bar.
 */
const BROWSE_FAILURES = {
  no_network: {
    title: "Reveille could not reach the network",
    remedy:
      "Check that this PC is online. A firewall blocking the master server's TCP connection can cause this too.",
  },
  master_unreachable: {
    title: "The master server could not be reached",
    remedy: "It is run by the community and is sometimes down. Try again in a few minutes.",
  },
  master_unreadable: {
    title: "The master server sent a reply Reveille could not read",
    remedy: "Nothing on this PC caused it. Try again; the reply may have been cut short.",
  },
  internal: {
    title: "The server list could not be built",
    remedy: null,
  },
};

export function browseFailureText(failure) {
  const known = BROWSE_FAILURES[failure?.kind] ?? BROWSE_FAILURES.internal;
  return { ...known, detail: failure?.detail ?? "" };
}
