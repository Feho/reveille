# UX standards

The outside evidence Reveille's interface is measured against. Researched 26 Aug 2026.

`docs/ui.md` is still authoritative for *what Reveille's interface is*. This file is the
external bar it should clear, so a disagreement between the two is a decision to make, not a
defect to fix silently. Where this file overturned something, `docs/ui.md` records the change and
the reason.

Every rule carries a confidence tag. **The tag is the point** — a rule tagged *Common practice*
may be argued with; one tagged *Standard* may not.

| Tag | Meaning |
|---|---|
| **Standard** | A normative specification, law, or platform requirement. |
| **Strong evidence** | Replicated research or large-sample usability data. |
| **Common practice** | Widely done, weakly evidenced. Follow for familiarity, not proof. |
| **Contested** | Practitioners genuinely disagree, or newer work overturned older doctrine. |

---

## 0. Widely repeated advice that does not hold up

Recorded first because these are the rules a well-meaning contributor is most likely to apply to
Reveille by reflex. **None of them should be adopted here.**

- **Skeleton screens are not established as better than spinners.** *Contested.* Viget's
  controlled test (n=136, identical durations) found skeletons scored *worst* on perceived speed —
  59% agreement versus 74% for a spinner. Reveille's determinate counters beat both, and a
  skeleton would imply rows are coming that may never arrive.
  <https://www.viget.com/articles/a-bone-to-pick-with-skeleton-screens/>
- **Zebra striping has essentially null evidence.** *Contested.* Two A List Apart studies found no
  significant accuracy gain. Use it if it looks right; never claim it aids scanning.
  <https://alistapart.com/article/zebrastripingdoesithelp/>
- **The F-pattern is a failure signal, not a layout target.** *Strong evidence.* NN/g's own
  updated position: the F appears when text is unformatted and motivation is low — "the pattern
  represents a design failure."
  <https://www.nngroup.com/articles/f-shaped-pattern-reading-web-content/>
- **Readability formulas are rejected by the authors of the plain-language canon.** *Contested.*
  Jarrett and Redish, Nov 2025, give seven reasons to abandon them. Use one as a smell test, never
  as a target or a gate.
  <https://www.effortmark.co.uk/readability-formulas-seven-reasons-to-avoid-them-and-what-to-do-instead/>
- **GOV.UK's own "25 words" evidence was misreported.** *Contested.* The source blog post carries
  a correction in its comments. The rule is still good practice; the numbers behind it are not
  evidence.
- **Padding time estimates so as not to disappoint users.** *Contested, and rejected here.*
  Nielsen recommends it. It is a deliberate small lie, and it is out of character for a product
  that refuses to say "joined" when it means "launched". See section 5.

---

## 1. Honesty and hedged information

The most important section, and the one where the evidence is most counter-intuitive.

**1.1 Numeric uncertainty costs almost no trust. Verbal uncertainty costs more.**
*Strong evidence.* van der Bles et al., PNAS 117(14):7672-7683 (2020) — five experiments including
a field experiment on the BBC News site. People do perceive more uncertainty when it is
communicated, as they should, but trust drops only slightly, and the drop concentrates in
**verbal** hedges rather than numeric ranges.
<https://www.pnas.org/doi/abs/10.1073/pnas.1913678117>

**This is the governing rule for Reveille's state labels.** A hedge must be a measurement or a
stated fact, never a mood word:

| Never | Always |
|---|---|
| "may be inaccurate" | "42 clients. Bots counted separately: 6." |
| "appears to be online" | "Replied 3 seconds ago." |
| "possibly compatible" | "3 of 5 maps found. 2 missing." |
| "Can't tell" | "Map list not published" |

**1.2 The instinct to soften a measurement into words makes it less trusted, not more.**
*Strong evidence.* Senders systematically prefer verbal probability expressions and wrongly
believe they help low-numeracy readers; recipients making consequential decisions prefer numbers,
and different readers assign wildly different values to the same word.
<https://www.cell.com/trends/cognitive-sciences/fulltext/S1364-6613(22)00060-2>

**1.3 A closed, published lexicon makes a hedge legible.** *Standard within its field (IPCC),
transferable.* IPCC calibrated language works because the same word always means the same thing
and the mapping is stated where the reader can find it. Reveille's version is evidential rather
than probabilistic, which is easier. The lexicon lives in `docs/rules.md` and is surfaced in the
app by the **What these words mean** panel.

**1.4 Ambiguous referents are the number one failure mode.** *Strong evidence.* "30% chance of
rain" is misread as 30% of the area, or 30% of the time. Every number must name what it measures:
not "Ping 48" but "Ping 48 ms to you"; not "42 players" but "42 clients, 6 bots".
<https://nap.nationalacademies.org/read/11699/chapter/4>

**1.5 Prefer counts to percentages.** *Strong evidence.* Icon arrays and natural frequencies
outperform percentages for low-numeracy readers. "2 of 5 maps missing", never "60% compatible";
"12 of 214 did not reply", never "94% response rate".

**1.6 State what the check examined and what it cannot see.** *Strong evidence.* Amershi et al.,
CHI 2019, guidelines G1 ("make clear what the system can do") and G2 ("make clear how well").
<https://www.microsoft.com/en-us/research/wp-content/uploads/2019/01/Guidelines-for-Human-AI-Interaction-camera-ready.pdf>

**1.7 Scale explanation to stakes, not uniformly.** *Common practice (Google PAIR).* A bookmarked
server launched twenty times needs no essay. A 400 MB download onto a server never played needs
full provenance.

**1.8 Overconfident phrasing causes overreliance; excessive hedging causes distrust.**
*Strong evidence, genuine tension.* The resolution: **be certain about what was measured and
explicit about what was not.** "Reveille found 3 of the 5 maps this server reports" is a confident
sentence about a limited fact. "This server may or may not work" is a hesitant sentence, and
hesitant sentences erode trust.

**1.9 A novel visual encoding of uncertainty will be misread.** *Strong evidence.* The hurricane
"cone of uncertainty" is a decade-long case study in a graphic experts find clear and the public
systematically misreads. Do not invent one for compatibility confidence.

**1.10 "We looked and found nothing" and "we did not look" are different words, different icons,
and different remedies.** Collapsing them is the specific dishonesty Reveille exists to avoid, and
the specific confusion users report about every other server browser (section 7).

---

## 2. Plain language

**2.1 One term per concept, one concept per term.** *Standard (US federal plain-language guidance,
Plain Writing Act 2010).* "You will confuse your audience if you use different terms for the same
concept... Don't feel that you need to use synonyms to make your writing more interesting."

This is the cheapest rule to enforce and the most violated. Reveille's fixed term list lives in
`docs/rules.md`; violations are greppable.

The rule runs both ways: because Reveille deliberately distinguishes *launch* from *join* and
*listed* from *replied*, those are separate concepts and must never be substituted for each other
either.

**2.2 Split sentences over 25 words; paragraphs of at most 5 sentences.** *Standard within UK
government.* For interface copy treat 2 to 3 sentences as the working limit. Evidence caveat in
section 0.

**2.3 Avoid negative contractions.** *Standard within UK government.* "Use contractions like
'you'll', but avoid negative contractions like 'can't' or 'don't', as many users find negative
contractions hard to read or misread them." Write **cannot**, not can't. Cheap, and it matters for
a substantially non-native audience.
<https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/>

**2.4 Reading level: lower secondary is the floor.** *Standard (WCAG SC 3.1.5, Level AAA).* Where
text needs reading ability beyond lower secondary education — roughly grades 7 to 9 — supply
supplemental content. It is the only normative reading-level number that exists. GOV.UK's
reading-age-9 target is *Contested* and three to four grades stricter; treat it as an aspiration.

**2.5 Keep the domain nouns the audience already owns.** *Standard within UK government.* GOV.UK's
distinction between jargon and necessary specialist language is clarity, not vocabulary size. Keep
*ping*, *map*, *mod*, *server*. Explain *BSP checksum*, *search path*, *rotation*, *master list*
on first use, or remove them. **Simplifying away the words a returning MOHAA player already knows
is the wrong correction.**

**2.6 Active voice, concrete subjects.** *Strong evidence.* "Reveille could not reach this server"
beats "The server was unreachable" — and is more honest, because it names who did the measuring.

**2.7 No idiom, wordplay, cultural reference, or humour.** *Strong evidence (direction).*
Non-native speakers outnumber native English speakers roughly four to one. "The server's call"
does not survive translation.

**2.8 Acceptance rule for the string catalogue.** Every player-facing string: 25 words or fewer;
2 sentences or fewer; no negative contraction; every domain term drawn from the fixed list; no
idiom; the first clause carries the meaning; any hedge is a measurement, not a mood word.

---

## 3. Explanatory text and tooltips

**3.1 Nothing meaningful goes in a `title` attribute.** *Standard (WCAG SC 1.4.13 Content on Hover
or Focus, Level AA) plus Strong evidence.* Hover content must be **dismissible** (Esc, without
moving the pointer), **hoverable** (the pointer can enter it), and **persistent** (it stays until
dismissed or invalidated). A `title` fails all three by construction, and is additionally
keyboard-unreachable, touch-unreachable, unstylable, truncated, and inconsistently announced by
assistive technology.
<https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html>

**3.2 Tooltips never carry task-critical information.** *Strong evidence.* NN/g's five guidelines
explicitly exclude field requirements, actionable instructions, and anything needing a long
explanation.
<https://www.nngroup.com/articles/tooltip-guidelines/>

**3.3 The three-way decision.**

- **Persistent inline text** — when the information changes the decision, or when omitting it
  would let the reader believe something false. Reveille's verdicts and caveats belong here.
- **Progressive disclosure** — when the information is explanatory rather than decisive: how the
  verdict was reached, which files are missing. Maximum 2 to 3 levels (*Strong evidence*; beyond
  that "users become disoriented", NN/g).
- **Tooltip** — glossary-grade definitions only, and only as a *duplicate* of something reachable
  another way.

**3.4 Budget explanatory text, because it will not be read.** *Strong evidence.* Weinreich et al.,
ACM Transactions on the Web 2008, n=45,237 page views: users read at most about 28% of words, and
realistically about 20%. Each additional 100 words buys roughly 4.4 seconds of attention. **The
caveat that must land goes in the first line, in the largest text, adjacent to what it
qualifies** — never in a paragraph beneath it.
<https://www.nngroup.com/articles/how-little-do-users-read/>

**3.5 Beginners plough, they do not scan.** *Strong evidence.* Low-literacy and non-native readers
read word by word, have a narrower visual field, and recover badly after an irrelevant first hit.
Short lines, front-loaded meaning, no long prose blocks — and a confusing first sentence costs more
here than it would with an expert audience.

**3.6 Hover timings.** *Strong evidence.* Feedback on entry within 0.1 s; dwell 0.3 to 0.5 s before
revealing; reveal within 0.1 s; keep visible for 0.5 s after the pointer leaves both the trigger
and the content.

---

## 4. Errors and empty states

**4.1 Cause plus remedy, in the reader's vocabulary.** *Strong evidence.* Place errors adjacent to
their source; scale severity to impact; never blame; preserve user input; suggest the correction.
<https://www.nngroup.com/articles/error-message-guidelines/>

**4.2 Banned from error copy.** *Standard within UK government.* Technical jargon; "forbidden";
"illegal"; **"please"**; **"sorry"**; error codes; **"valid" and "invalid"**; humour; and repeating
an example already shown as hint text.
<https://design-system.service.gov.uk/components/error-message/>

**4.3 No library, OS, or crate string ever reaches the player.** Classify at the boundary into a
closed set, each with fixed copy. Reveille already does this for engine failures
(`OpenMohaaFailureKind`); the browse and catalogue paths do not yet.

**4.4 Cold, empty, filtered-empty and failed are four distinct states.** *Strong evidence.* Showing
one when you mean another is the failure — users cannot tell error from empty from broken.

| State | Means | Copy shape |
|---|---|---|
| Cold | Reveille has not looked yet | "No servers listed yet. **[Find servers]**" |
| Swept, zero rows | Reveille looked and found nothing | "The master list returned no servers. Try again in a few minutes." |
| Filtered to zero | The filters excluded everything | "None of the 214 servers match. **[Clear filters]**" plus the active filters |
| Sweep failed | Reveille could not look | Classified cause and remedy — **and keep the previous rows, marked stale** |

**4.5 A failed sweep must not blank the table.** *Strong evidence.* Replacing good-but-stale data
with an empty screen destroys information for no reason. This is where an honest product beats the
conventional one.

**4.6 Never discard completed work on failure or cancellation.** A cancelled multi-file download
keeps what it got and says so: "Stopped. 2 of 3 maps were downloaded and kept."

**4.7 Every empty state offers a pathway, not only prose.** *Strong evidence.* NN/g's three rules:
communicate system status; put the learning cue in place ("Star a server to list it here"); give a
real button.
<https://www.nngroup.com/articles/empty-state-interface-design/>

---

## 5. Progress and latency

**5.1 The thresholds.** *Strong evidence (perceptual, so they do not age).* 0.1 s feels
instantaneous; 1 s is the limit of uninterrupted thought; 10 s is the limit of attention.

**5.2 What to show.** *Strong evidence.* Under 1 s, nothing. 1 to 2 s, acknowledge the action.
2 to 10 s, an indeterminate animation. Over 10 s, percent-done. **And percent-done is also correct
below 10 s when the work is a countable series** — which a 200-server sweep and a multi-file
download both are.
<https://www.nngroup.com/articles/progress-indicators/>

**5.3 Label the phase.** *Common practice, strong rationale.* Harrison et al. found that what users
hate most is a bar that **stalls**. A join flow whose phases run at different rates — resolve,
download, verify, install, launch — will appear to hang during verification unless each phase is
named and metered separately.

**5.4 Honest progress.** *Contested; Reveille deliberately diverges.* Always show completed over
total in the unit actually counted. Show measured throughput, not a smoothed estimate. Render a
time estimate only when recent samples have low variance, and label it as derived — "about 40
seconds left at the current speed". Never let a bar go backwards or rest at 99%; if the remaining
work is unbounded, switch that phase to indeterminate and say why.

**5.5 Cancel is present from the first frame,** actually aborts the network work rather than hiding
the dialog, and reports the resulting state.

**5.6 An explicit-refresh product must show data age.** *Common practice.* Every row on screen is a
claim about the past. Absolute timestamps, and stale data should look stale.

**5.7 Never stream progress into a live region.** *Standard-adjacent (ARIA practice).* A sweep that
announces each of 200 responses is a denial of service on a screen-reader user. Announce **start**,
**coarse milestones**, and **completion with the summary**.

---

## 6. Accessibility, the 2026 baseline

**6.1 WCAG 2.2 AA is the target.** *Standard.* W3C Recommendation 5 Oct 2023, updated 12 Dec 2024;
also ISO/IEC 40500:2025. WCAG 3.0 remains an early Working Draft and is **not** a usable target;
APCA was removed from that draft in July 2023 and is not normative.

**6.2 What 2.2 added that applies here.** *Standard.*

| SC | Name | Level | Requirement |
|---|---|---|---|
| 2.4.11 | Focus Not Obscured | AA | The focused component is not entirely hidden by a sticky header or the detail pane |
| 2.5.8 | Target Size (Minimum) | AA | 24 by 24 CSS px, or 24 px-diameter spacing |
| 3.2.6 | Consistent Help | **A** | Help mechanisms appear in the same relative order across views |
| 3.3.7 | Redundant Entry | **A** | Information already provided is not asked for again |

3.3.7 is the sleeper: making a player re-locate their game folder, or re-type a server address they
already bookmarked, is a **Level A** failure.

**6.3 Contrast.** *Standard.* Body text 4.5:1; large text (24 px or more, or 18.66 px bold) 3:1;
interface boundaries, states and focus indicators 3:1, **unrounded** — 2.999:1 fails. Disabled
controls are exempt, which is a trap (6.6).

**6.4 The WCAG 2 contrast formula is known to be flawed, and is still the one to use.**
*Contested.* It ignores stroke weight and overstates contrast for near-black colours, so dark
palettes that pass 4.5:1 can be functionally unreadable. Sanity-check dark themes with APCA; never
substitute it for the ratio.
<https://adrianroselli.com/2026/04/wcag3-contrast-as-of-april-2026.html>

**6.5 A table with selection and two-dimensional navigation must be `role="grid"`, not a static
table.** *Standard (ARIA APG).* In a static `<table>` **every** focusable descendant joins the page
tab sequence — with 200 rows that is unusable. `role="grid"` is a composite widget: **one tab
stop**, with arrow keys moving a roving focus.

Required keys: arrows between cells; `Home` and `End` for row ends; `Ctrl+Home` and `Ctrl+End` for
grid corners; `PageUp` and `PageDown` by viewport. Set `aria-rowcount` if rows are virtualised, or
screen readers announce only the rendered window. Prefer **roving tabindex** over
`aria-activedescendant` — real DOM focus gets `:focus-visible` and scroll-into-view for free.
<https://www.w3.org/WAI/ARIA/apg/patterns/grid/>

**6.6 Unavailable primary actions use `aria-disabled` and stay focusable.** *Contested.* Native
`disabled` removes the control from the tab order, so a keyboard user never finds it and is never
told why — and WCAG exempts disabled controls from contrast, so disabled states are legally allowed
to be illegible. For an action unavailable in *common* states, use `aria-disabled="true"`, keep it
focusable, meet 3:1 anyway, block the action in the handler, and put the reason in adjacent
persistent text.
<http://adrianroselli.com/2024/02/dont-disable-form-controls.html>

**6.7 Colour is never the only carrier.** *Standard (SC 1.4.1, Level A).* Test by rendering in
grayscale. A global colourblind filter does not satisfy this.

**6.8 Live regions.** *Standard (ARIA).* Default to `role="status"` with `aria-live="polite"`;
reserve assertive for blocking or expiring content. **Register the region empty in the initial
DOM**, then inject text — a region created and filled in the same tick often announces nothing. Do
not pair `aria-live="assertive"` with `role="alert"`.

**6.9 Layout survives 200% browser zoom and 200% Windows display scale.** *Standard (SC 1.4.4,
AA).* These are different code paths; test both.

**6.10 Test with Narrator, not only NVDA.** *Standard (platform guidance).* A Tauri app exposes UI
Automation through the webview bridge and behaves differently from a browser. Use Accessibility
Insights for Windows.

**6.11 Regulatory position, for the record.** *Standard.* EN 301 549 V3.2.1 (WCAG 2.1 AA) is still
the harmonised EU version; final draft V4.1.0 was published June 2026 and moves to WCAG 2.2 AA,
expected in the Official Journal around October 2026. The European Accessibility Act has been
enforceable since 28 June 2025, but a free hobbyist game launcher is very unlikely to fall in
scope, and microenterprises are exempt from the service obligations. **Build to 2.2 AA because it
is right, not because it is required.**

---

## 7. Windows, Tauri, and prior art

**7.1 Keyboard conventions Windows users will try.** *Standard (platform guidance).*
`F5` refresh, `Ctrl+F` find, `F6` and `Shift+F6` cycle panes, `Esc` cancels transient interface
without navigating back, `Home`/`End`/`PageUp`/`PageDown` in lists, `Space` invokes and `Enter`
activates, and arrow keys for inner navigation within a control group that is a single tab stop.

Initial focus goes on the most likely action, **never** on a control with an expensive or
destructive outcome.
<https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-interactions>

**7.2 Text is selectable and copyable.** *Standard (platform guidance).* "Whenever there's text in
an application, users expect that they can select and copy it." Server names, addresses and paths
must be copyable; chrome must not be. This is the correct resolution of the usual blanket
`user-select: none`.

**7.3 The webview tells.** *Common practice.* Suppress the WebView2 context menu and supply a real
app menu — on a row that means Bookmark, Copy address, Re-ask this server. Trap the browser
affordances that leak: whole-page reload, the browser find bar, Backspace navigation, file-drop
navigation. Restyle focus rings and scrollbars, keeping 3:1 and 24 px targets. Specify Segoe UI
Variable with an explicit fallback stack.

**7.4 "Are you sure?" does not work.** *Standard (Microsoft security guidance).* Users are
conditioned to click Yes. Destructive paths need a differently-labelled button naming the action
("Clear 14 launches"), a different gesture, or an undo.

**7.5 Windows install expectations.** *Standard (platform guidance).* Per-user install; no
elevation to install or to run; silent-install support; listed in Settings, Apps, Installed apps;
complete uninstall; **sign every binary, not only the installer** — without MSIX there is no
package signature, so binary signatures are what Smart App Control checks.
<https://learn.microsoft.com/en-us/windows/apps/get-started/best-practices>

**7.6 WebView2 is a real first-run dependency on Windows 10.** *Standard (platform fact).*
Preinstalled on Windows 11 only. The Tauri default installer downloads a bootstrapper and so
**needs internet at install time**; `offlineInstaller` embeds it at roughly +127 MB. Its absence
needs its own named error class.

### Prior art worth copying

- **Doomseeker** (Zandronum) — the closest analogue. Filters apply immediately; **ping is a
  threshold filter, not only a sort**; favourites via right-click; double-click auto-downloads the
  needed PWADs and connects. Critically, it **auto-downloads optional content but requires the base
  game data up front, stated as a precondition** — exactly Reveille's engine-versus-game-data
  boundary, already solved for the same kind of audience. *Genuinely good.*
- **Vortex's Dependency Health Check** — warn only when resolution is unambiguous; say "enable it"
  rather than "install it" when the dependency exists but is switched off; **suppress warnings
  already being resolved and clear them automatically**. *Genuinely good* — it prevents the warning
  that outlives its cause and trains users to ignore warnings.
- **Mod Organizer 2** — when the system cannot decide correctly, expose the decision rather than
  guessing quietly. *Genuinely good.*

### Prior art's documented failures

- **Steam's server browser**: servers that are online and joinable by direct IP simply do not
  appear in the filtered list, with no explanation. The user's model is "the list is the world", so
  a known-good server's absence reads as "the server is dead". **Reveille's registered-versus-
  answered line is the antidote to the best-documented failure in this product category, and must
  therefore be legible.** Removing the total server count also drew immediate complaints.
- **Shipping sort without filter** (Hell Let Loose, Squad): sorting by clients surfaces full 250 ms
  servers; sorting by ping surfaces empty nearby ones. Neither alone works — filter to playable,
  then sort by populated.
- **Conflict warnings that are true but not decision-relevant** (Vortex): users learn to dismiss
  them, at which point an honest verdict is worth less than none. **Rank by consequence, not by
  detectability.**
- Launcher interfaces in this category are generally weak — Epic has publicly said its own launcher
  "sucks". Treat none of it as a design reference; the bar is low.

*Standing caveat: there is no peer-reviewed literature on server-browser UX. Everything in this
subsection is documented behaviour or aggregated complaint — useful as convention and as a
catalogue of observed failure, not as evidence.*

---

## 8. Checklist

A scoring rubric. **[S]** marks rules where a failure is a defect, not a preference.

### First run
1. The prerequisite — you must already own the game — is stated before any effort is spent.
2. The install is auto-detected and offered for confirmation, not typed by hand.
3. The engine download is skippable; setup is re-enterable later.
4. Nothing already provided is asked for twice. **[S: WCAG 3.3.7, A]**
5. No elevation to install or to run. **[S: Windows]**
6. A missing WebView2 on Windows 10 is a named, actionable error.
7. Cold launch to a populated table is under 60 s of user-attributable work.

### Server table
8. Every in-row target is 24 by 24 px or larger, or spaced so a 24 px circle hits nothing else.
   **[S: 2.5.8, AA]**
9. Text left-aligned; counts and ping right-aligned with tabular figures.
10. The decision-driving column is leftmost; no horizontal scroll at 1280 px.
11. Sortable headers are real buttons; `aria-sort` on the active header only. **[S: ARIA]**
12. Each column's initial sort direction is chosen for meaning.
13. Sort, scroll, selection and filters survive a sweep.
14. Search shows a match count, such as "38 of 214".
15. Ping is available as a threshold filter, not only a sort.
16. `role="grid"` with roving tabindex — one tab stop, arrows navigate. **[S: ARIA APG]**
17. `aria-rowcount` is set if rows are virtualised. **[S: ARIA]**
18. No state is carried by colour alone. **[S: 1.4.1, A]**
19. Row state is a count or a noun, never a traffic light.

### Copy
20. Every string is 25 words or fewer and 2 sentences or fewer.
21. The fixed term list is enforced, greppable, with no synonyms.
22. *Launch* and *join*, *listed* and *replied*, are never substituted.
23. No negative contractions; active voice; no idiom or humour.
24. Specialist terms are explained on first use, or removed.
25. Domain nouns the audience owns are kept, not simplified away.

### Errors and empty states
26. No library, OS or crate string reaches the player.
27. Every error states cause and remedy; none says "please", "sorry", "invalid", or a code.
28. Cold, empty, filtered-empty and failed are four distinct states.
29. A failed sweep keeps the previous rows, marked stale.
30. No completed work is discarded on failure or cancellation.

### Progress
31. Nothing under 1 s gets an indicator.
32. The sweep shows a determinate count, not a spinner.
33. Each phase of the join flow is named and metered separately.
34. No padded estimate; any estimate is explicitly qualified.
35. Cancel is present from the first frame and aborts real work.
36. Announcements are start, milestones, summary — never per item. **[S-adjacent: ARIA]**
37. The live region is registered empty before being populated. **[S: ARIA]**

### Honesty
38. Every hedge is a measurement, never a mood word. **[Strong evidence]**
39. Every number names its referent and its unit.
40. Clients and bots are adjacent, labelled, and never summed.
41. Anything perishable carries an absolute timestamp.
42. Counts, not percentages.
43. The detail pane says what was examined and what cannot be seen.
44. A "What these words mean" panel is reachable from the same place on every view.
    **[S: 3.2.6, A]**
45. Explanation depth scales with stakes.

### Accessibility
46. Text 4.5:1 or better; large text 3:1 or better. **[S: 1.4.3, AA]**
47. Boundaries, states and focus 3:1 or better, unrounded. **[S: 1.4.11, AA]**
48. Focus is always visible and never entirely hidden. **[S: 2.4.7 and 2.4.11, AA]**
49. Unavailable primary actions use `aria-disabled`, stay focusable, and state the reason.
50. Nothing meaningful lives in a `title`. **[S: 1.4.13, AA]**
51. 200% browser zoom and 200% display scale both hold. **[S: 1.4.4, AA]**
52. Tested with Narrator and Accessibility Insights.

### Windows and Tauri
53. `F5` refreshes, `Ctrl+F` searches, `F6` cycles panes, `Esc` cancels without navigating.
54. Initial focus is on search or the table, never on the launch button.
55. Names, addresses and paths are selectable; chrome is not.
56. The webview context menu is suppressed and a real row menu replaces it.
57. Dark, light and high-contrast themes are all correct.
58. Destructive confirmations use a specific verb, not "Yes". **[S: Microsoft]**
