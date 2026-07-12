# Status Updates & Bomb Icon Animation UX Plan

**Project:** Bomb Code (Grok Build Tauri control panel)  
**Date:** 2026-07-11  
**Status:** Research + planning (no implementation in this pass)  
**Owners:** Frontend / design system  
**Related code:**
- `frontend/app.js` — `noteTurn`, `updateBombChrome`, `PHASE_*`, event handlers
- `frontend/bombs.css` — `.px-bomb` moods + keyframes, `.turn-dock`
- `frontend/styles.css` — layout chrome, status pill
- `frontend/index.html` — turn dock, composer phase, activity “Now”, brand sub

---

## 1. Goal

Make **turn status** and **pixel-bomb animation** feel like one coherent language—not five independent widgets that each re-explain “thinking.”

**Success feels like:**
1. User always knows *what the agent is doing right now* in one glance.
2. The bomb icon is a **mood instrument**, not decoration or a second status line.
3. Motion is calm when waiting, sharp on phase change, celebratory only on real completion.
4. Facts (tokens, tools, elapsed, stall) are trustworthy; flavor never competes with them.
5. Surfaces (center dock, composer, right rail, status pill, brand) **agree** without parroting.

---

## 2. Research inputs

### 2.1 What good agent UIs do

| Product / pattern | Status model | Motion | Integration lesson for Bomb Code |
|-------------------|--------------|--------|----------------------------------|
| **Claude / Claude Code** | Single phase line + streaming body; tool blocks inline | Soft pulse / spinner on wait; no competing headers | **One primary activity line**; stream is the proof of life |
| **Cursor** | Compact “Generating…” + file/tool chips | Subtle shimmer; phase changes are discrete | Prefer **chips over paragraphs** for tools |
| **Codex / OpenAI apps** | Timeline of steps; current step emphasized | Minimal idle motion | **Stage rail** works if stages are real events, not theater |
| **Linear / Vercel** | Status = badge + short verb; detail on hover/expand | Almost no loop animation | **Reduce loop intensity**; reserve motion for transitions |
| **Game HUD (fuse / payload metaphor)** | Instrument cluster: fuse = danger/energy, gauge = progress | Fuse spark only when “armed”; boom only on impact | Bomb moods should map to **arm / burn / detonate / dud**, not generic spinner clones |

### 2.2 Motion design principles (applicable)

From general product animation practice (and our current excess):

1. **Signal over spectacle** — Looping animation answers “is it alive?” Transition animation answers “what changed?”
2. **One tempo hierarchy** — At most one “loud” loop on screen (e.g. dock bomb). Everything else is static or 10–20% intensity.
3. **Phase transitions > perpetual motion** — 200–400ms mood morph on phase change; long loops only for true wait states.
4. **Respect reduced motion** — Already partially present (`prefers-reduced-motion`); make it first-class: static icon + text only.
5. **Don’t animate meaning twice** — If stage rail shows “TOOLS,” bomb doesn’t need a different story.

### 2.3 Status copy principles

1. **Primary = verb + object** (“Reading `client.rs`”, “Writing reply”).
2. **Secondary = metrics** (elapsed, chars, tool count)—tabular, muted.
3. **Tertiary = flavor** (bomb puns)—optional, never alone.
4. **Stall is a first-class state** — Not “Still working” as a euphemism; say *why* quiet (no token, waiting on tool, permission, ACP silence).
5. **Trust after ACP fix** — With stream events fixed, UI can prefer *live evidence* over optimistic “thinking forever.”

---

## 3. Current-state audit (as of 2026-07-11)

### 3.1 Turn state machine

```
idle → send → think → tools ⇄ reply → done → idle
                 ↘ wait ↗
                 ↘ error ↗
```

Implemented in `emptyTurn()` / `noteTurn()` / `PHASE_LABEL` / `PHASE_MOOD`.

**Strengths**
- Ranked phase advance prevents accidental regression (mostly).
- `lastSignalAt` enables stall detection (`STALL_MS = 25s`).
- Stage rail (Send → Think → Tools → Reply → Done) is a good skeleton.

**Weaknesses**
- `tools` and `reply` fight: after a completed tool with `streamChars`, code jumps back to `reply`, then next tool re-enters `tools`—stage rail thrash.
- No phases for: **connecting**, **loading brain**, **permission**, **terminal/host tool**, **compact/resume**.
- “tokens” in dock detail are often **char counts / context tokens**, not completion tokens—labeling confuses.
- Flavor rotates every 2s (`startPhraseCycle`) while user is trying to read facts.

### 3.2 Surfaces that show status (duplication map)

| Surface | Element IDs | Driven by | Bomb mood | Problem |
|---------|-------------|-----------|-----------|---------|
| Brand subline | `#brand-sub` | `updateBombChrome` | — | Mirrors phase; low value when dock visible |
| Status pill | `#status-pill`, `#status-bomb`, `#status-text` | `setStatus` + turn override | Yes | Host health vs turn phase **collide** |
| Turn dock | `#turn-dock`, `#turn-bomb`, stages, meter | `updateBombChrome` | Yes (primary) | Best surface, but dense + flavor noise |
| Composer chip | `#composer-phase` | same | Yes (xs) | Third copy of phase + bomb |
| Activity “Now” | `#now-panel`, `#activity-bomb` | same | Yes | Fourth copy |
| Thread list badges | `.thread-item .badge` | `renderThreads` | Yes | Session-level OK |
| Timeline events | `#event-feed` | `pushEvent` | Yes per line | Bomb on every line is noisy |
| Transcript live row | stream caret / live block | render transcript | Yes | Fine if dock is primary |

**Integration gap:** Same `mood` + `PHASE_LABEL` painted 3–5 times. When stalled, user sees “Still working” + rotating puns + indeterminate meter + bouncing bomb + stage “THINK” active—**visual shouting**.

### 3.3 Bomb animation system

**Moods** (`.px-bomb.mood-*`):  
`idle | ready | thinking | running | tooling | stream | boom | error | wait`

**Mapped from turn phase** (`PHASE_MOOD`):

| Phase | Mood | Loop character |
|-------|------|----------------|
| send / think | thinking | Bounce + fuse spark |
| tools | tooling | Step-spin + chips |
| reply | stream | Breathe + side dots |
| wait | wait | Pulse (yellow) |
| done | boom | One-shot pop |
| error | error | Wobble + smoke |
| stall override | running | Hard shake |

**Strengths**
- Distinct pixel language; brand-aligned.
- Pseudo-elements for fuse/sparks without extra assets.
- Size scale `xs→xl` reuses one component.

**Weaknesses**
1. **Too many simultaneous loops** — dock + composer + activity + status all animate at full intensity.
2. **Mood thrash** — tool↔reply flips spin↔breathe every few hundred ms.
3. **`running` (shake)** reserved for stall but feels like panic/error, not “still alive.”
4. **`boom` on done** is good, but often followed by instant idle—user misses the reward.
5. **No transition choreography** — class swap is instant; CSS restarts keyframes hard.
6. **Single logo PNG** — all moods are filters + motion of one asset; no silhouette change for “armed vs spent.”
7. **Meter bar** is indeterminate always—doesn’t reflect stream progress when we have chars/tools.

### 3.4 Event → turn wiring (after ACP stream fix)

With `session/update` actually delivered (JSON-RPC parse fix), expected signal quality is high:

| Event | Should advance | UI emphasis |
|-------|----------------|-------------|
| user send | send → think | dock appears |
| agent_thought | think | fuse mood; thought preview |
| agent_message | reply | stream mood; preview = speech |
| tool_call start | tools | tool name chip |
| tool_call done | tools or reply | soft tick, not full boom |
| permission | wait | wait mood; CTA |
| session idle / prompt complete | done | boom once, then settle |
| long silence | stall *substate* | calmer, not shakier |

**Still missing for elegance:** host tool breadcrumbs (`term` channel), brain-load, and “first token” should be explicit substates, not only detail strings.

---

## 4. Problems to solve (ranked)

### P0 — Coherence
1. **Single source of truth** for “what is happening now.”
2. **One primary motion focus** (dock bomb); satellites use static or micro-mood.
3. **Stop flavor from competing** with stall/error facts.

### P1 — Honesty
4. Stall UX that distinguishes: model silence vs tool hang vs waiting on user.
5. Progress that uses real signals (chars, tools, stages) when available.
6. Accurate labels (context tokens ≠ reply tokens).

### P2 — Delight (on brand)
7. Boom only on meaningful completion (turn done, wave clear, not every tool).
8. Smoother mood transitions (crossfade / fuse lengthen).
9. Stage rail only lights stages that actually occurred this turn.

### P3 — Polish
10. Reduced-motion path excellence.
11. Multi-session: selected turn vs background session indicator without five bombs dancing.
12. Performance: avoid re-`innerHTML` of bomb sprites every chrome tick.

---

## 5. Design direction

### 5.1 Metaphor: one fuse, one charge

Treat the pixel bomb as a **fuse instrument** for the *selected* session’s active turn:

| Instrument reading | Meaning |
|--------------------|---------|
| Cold / dim | Idle — no charge |
| Fuse lit, slow pulse | Thinking / waiting for first signal |
| Fuse hot, steady | Streaming reply (alive, productive) |
| Mechanical ticks | Tools (work, not panic) |
| Amber hold | Needs user (approval / input) |
| Detonation flash | Turn complete (success) |
| Smoke / gray | Error or cancel |

**Rule:** Only the **turn dock bomb** (or full-screen empty state hero) may use high-energy loops. All other bombs are **badges** (static mood tint or 1fps twinkle).

### 5.2 Copy hierarchy (visual weight)

```
[BOMB]  Thinking · 1m 12s          ← primary (phase + elapsed)
        2 tools · last: read_file  ← secondary (facts)
        “Checking sandbox path…”   ← tertiary (preview, 1 line)
        ── fuse meter / stages ──  ← structural
        packing powder             ← flavor, optional, 9–10px, can hide
```

When stalled:

```
[BOMB]  Quiet · 32s no signal
        Last: tool read_file (running)
        Waiting on agent stream or host tool
```

### 5.3 Surface roles (integration contract)

| Surface | Role | Shows bomb? | Shows full detail? |
|---------|------|-------------|--------------------|
| **Turn dock** | Primary turn presence | Yes, animated | Yes |
| **Composer chip** | Peripheral reminder while typing | Static tint only | Phase + elapsed only |
| **Activity “Now”** | Optional mirror for right-rail users | Static or xs loop | Compact facts |
| **Status pill** | **Host/app health only** (ready / offline / auth) | Static | No turn monologue |
| **Brand sub** | Idle product name; live = elapsed only | No | No phase essay |
| **Timeline** | Historical log | No bomb per line (or only on errors) | Event text |
| **Thread list** | Session status | Static badge | Status word |

---

## 6. Proposed system design

### 6.1 `TurnPresence` model (frontend)

Replace ad-hoc `state.turn` patches with an explicit presence object:

```ts
type TurnPhase =
  | "idle"
  | "send"
  | "think"
  | "tools"
  | "reply"
  | "wait"
  | "done"
  | "error";

type StallKind =
  | null
  | "no_first_signal"   // after send, nothing yet
  | "stream_gap"        // had signals, then quiet
  | "tool_hang"         // last tool still running / no completion
  | "awaiting_user";    // wait phase

interface TurnPresence {
  phase: TurnPhase;
  startedAt: number | null;
  lastSignalAt: number | null;
  stall: StallKind;
  // facts
  promptChars: number;
  thoughtChars: number;
  replyChars: number;
  contextTokens: number | null;  // from ACP _meta.totalTokens if present
  toolCount: number;
  toolsActive: number;
  lastTool: string | null;
  lastToolStatus: string | null;
  preview: string;         // latest agent speech snippet
  thoughtPreview: string;
  note: string;            // human reason for wait/error
  stagesSeen: Set<"send"|"think"|"tools"|"reply">;
  // presentation
  flavorEnabled: boolean;  // default false until user opts in, or only after 8s
}
```

**Derivation rules**
- `stall` computed every chrome tick from phase + lastSignal + lastToolStatus.
- Phase transitions emit a **one-shot** `transition: "enter_tools" | "first_token" | "complete"` for CSS classes lasting 300–500ms.
- `stagesSeen` only marks stages that actually fired—no fake “tools done” if no tools.

### 6.2 Animation architecture

#### A. Intensity tiers

| Tier | Where | Motion |
|------|-------|--------|
| **0 Static** | List badges, timeline, status pill | Color only |
| **1 Ambient** | Composer chip, activity header | 3–4s breathe, opacity only |
| **2 Active** | Turn dock bomb | Mood loop as today but toned down |
| **3 Peak** | Dock on boom/error; 400ms | One-shot keyframes |

#### B. Mood retune (specific)

| Mood | Change |
|------|--------|
| `thinking` | Slower bounce (1.2–1.4s); fuse spark every ~0.6s not 0.35s |
| `stream` | Prefer **steady glow** over bounce; optional soft scale 1.0→1.03 |
| `tooling` | Replace spin with **short nod** (rotate ±3°) + chip blink |
| `running` (stall) | **Do not shake.** Use `thinking` + amber border + “Quiet” label |
| `boom` | Hold `mood-boom` 900–1200ms before idle; dock stays visible |
| `wait` | Fuse twinkle only; no scale pulse (reduces anxiety) |

#### C. Transition class API

```html
<span id="turn-bomb" class="px-bomb md mood-stream is-entering">
```

CSS:

```css
.px-bomb.is-entering img {
  animation: bomb-mood-enter 0.35s ease-out both;
}
```

JS: on phase change, add `is-entering`, remove after `animationend` or timeout. Prevents hard keyframe restart feel.

#### D. Meter honesty

- **Indeterminate** only when `phase ∈ {send, think}` and no chars/tools yet.
- **Determinate-ish** when reply streaming: map `replyChars` through a soft asymptotic bar (never claim 100% until done).
- **Segmented** when tools: N segments for tools started this turn (optional v2).

### 6.3 Status string builder (single function)

```ts
function formatPresence(p: TurnPresence): {
  title: string;      // "Thinking" | "Quiet" | "Using tools"
  subtitle: string;   // facts line
  preview: string;    // optional
  mood: BombMood;
  tier: 0|1|2|3;
}
```

All surfaces call this. No bespoke “Still working” forks in three places.

**Title examples**
- think + no signal: `Thinking`
- think + stall no_first_signal: `Quiet`
- tools + lastTool: `Running · read_file`
- reply: `Writing`
- wait: `Needs approval`
- done: `Done`

### 6.4 Multi-session rules

- **Selected session** owns dock + composer chip.
- **Other running sessions**: thread list badge only (`RUNNING` + static bomb).
- Activity header bomb: **ambient** if any session busy; not full `running` shake.
- Switching sessions: swap presence from that session’s store (requires per-session `TurnPresence` map—see §8).

### 6.5 Accessibility

- `aria-live="polite"` on dock title only (not every flavor rotation).
- Announce phase changes, not every char.
- `prefers-reduced-motion: reduce` → all moods static; dock border color still changes; meter becomes solid bar by phase color.
- Don’t rely on color alone for error (icon + “Failed” text).

---

## 7. Information architecture (before / after)

### Before (noise)

```
Brand: Thinking · 4m
Pill:  Thinking · 4m
Dock:  Still working · flavor · detail · stages · meter · preview
Composer: Thinking · 4m + bomb
Activity: Thinking + same detail
Timeline: bomb per event
```

### After (integrated)

```
Brand: Grok Build panel          (or "Live · 4m" only if dock collapsed)
Pill:  Ready / Offline / Auth    (host only)
Dock:  [animated bomb] Writing · 4m
       1.2k reply chars · 3 tools
       preview line
       stages + honest meter
Composer: Writing · 4m           (static bomb tint)
Activity: Writing · 4m           (mirror, static bomb)
Timeline: text-first; bomb only for errors / boom completions
```

---

## 8. Implementation plan (phased)

### Phase A — Presence model & single formatter (1–2 days)

**Scope**
- Introduce `TurnPresence` (+ per-session map keyed by session id).
- Implement `formatPresence()` + stall derivation.
- Route `noteTurn` / event handlers through presence updates only.
- Fix label: `replyChars` vs `contextTokens`.

**Exit criteria**
- All surfaces read from `formatPresence`; grepping `PHASE_LABEL[` only in that module.
- Unit-testable pure functions for stall + phase rank (js or extracted module).

### Phase B — Surface demotion (1 day)

**Scope**
- Status pill: stop overriding with turn phase when host is healthy.
- Brand sub: idle name; optional compact live elapsed.
- Composer / Activity: static bomb tier 0–1.
- Timeline: remove default bomb icons (keep for err/ok milestones).

**Exit criteria**
- At most one tier-2 animated bomb visible during a turn.
- No triple identical “Thinking · Xm” strings.

### Phase C — Animation retune (1–2 days)

**Scope**
- CSS: slow thinking, no stall-shake, softer tooling, stream glow.
- `is-entering` transition class.
- Boom hold 1s on done.
- Meter modes: indeterminate / soft-progress / complete.
- Flavor: off by default or only after 8s quiet + not stalled; slower cycle (5–8s).

**Exit criteria**
- Reduced-motion QA checklist green.
- Side-by-side video: old vs new 30s think→tool→reply→done.

### Phase D — Stage rail honesty & tool chips (1–2 days)

**Scope**
- `stagesSeen` gating.
- Optional compact tool chip row under dock detail (last 3 tools).
- Permission wait: clear CTA copy (“Approve in timeline / auto-approved”).

**Exit criteria**
- Stage “TOOLS” never active if zero tools this turn.
- Tool thrash doesn’t flicker stage colors >2Hz (debounce 150–200ms).

### Phase E — Multi-session presence (1–2 days)

**Scope**
- `Map<sessionId, TurnPresence>`.
- On select session, dock binds that presence.
- Background busy indicator on thread cards only.

**Exit criteria**
- Two sessions running: only selected dock animates; other shows badge.

### Phase F — Optional asset evolution (later)

**Scope**
- 2–3 bomb sprites (idle / lit / spent) if single PNG limits expression.
- Micro confetti limited to dock bounds on boom (CSS only).

**Exit criteria**
- Still pixelated; no heavy Lottie/video.

---

## 9. Concrete CSS/JS change list (for implementers)

### `frontend/app.js`
- [ ] Extract `presence.js` (or section) with pure helpers.
- [ ] `updateBombChrome` → thin renderer from `formatPresence`.
- [ ] Debounce phase→mood updates (min 120ms between mood class changes except wait/error/done).
- [ ] Per-session presence map; hydrate idle on session switch.
- [ ] `pushEvent`: drop bomb html by default; `milestone: true` for boom/error.
- [ ] Flavor timer: 6s; skip when `stall !== null`.
- [ ] On `done`: `setTimeout` 1000ms before `idle`; keep dock visible.

### `frontend/bombs.css`
- [ ] Retune keyframe timings (§6.2.B).
- [ ] Remove/repurpose `bomb-shake` for stall.
- [ ] Add `.px-bomb.tier-ambient` reduced opacity animation.
- [ ] Add `.is-entering` enter animation.
- [ ] `.turn-dock-meter[data-mode="progress"]` without infinite translate.
- [ ] Strengthen reduced-motion block.

### `frontend/styles.css` / `index.html`
- [ ] Dock layout: title row facts-first; flavor optional `data-flavor="on"`.
- [ ] Composer chip styles for static bomb.
- [ ] Status pill copy slots: host only.

### Tests / manual QA
- [ ] Prompt → first thought <1s mood think.
- [ ] First reply char → stream mood once (no flip-flop).
- [ ] Tool spam: mood stays tooling until 300ms after last tool event.
- [ ] Stall at 25s: title “Quiet”, no shake.
- [ ] Done: boom visible ≥800ms.
- [ ] Reduced motion: no loops.
- [ ] Two sessions: only selected animates.

---

## 10. Metrics (how we know it’s better)

| Metric | Baseline (qualitative) | Target |
|--------|------------------------|--------|
| Distinct status strings on screen during think | 3–5 identical | 1 primary + ≤2 peripheral |
| Animated bomb instances (tier 2+) | 3–4 | 1 |
| Mood class changes / 10s tool-heavy turn | High thrash | ≤6 transitions |
| Time boom visible on success | Often 0–200ms | ≥900ms |
| User can answer “what is it doing?” in 1s | Often no | Yes in hallway test (n=3) |
| Stall false “panic shake” | Yes | No |

---

## 11. Risks & open decisions

| Topic | Options | Recommendation |
|-------|---------|----------------|
| Flavor copy | Always / after delay / settings toggle | **After 8s, hide when stalled**; default on for brand |
| Status pill ownership | Host-only vs dual-mode | **Host-only** |
| Tool↔reply flicker | Debounce vs sticky “tools until reply chars for 500ms” | **Sticky tools while any tool in-flight** |
| Per-session presence | Now vs later | Phase E after A–C |
| New bomb art | Keep logo vs multi-sprite | Keep logo through Phase C; revisit F |

**Does not require decision before A–C:** exact fuse colors (keep CSS vars).

---

## 12. Out of scope (this plan)

- Backend ACP / event bus changes (already fixed stream delivery path).
- Replacing bomb brand with abstract spinner.
- Full redesign of three-column layout.
- Sound design (optional later: soft tick on tool, low boom on done).

---

## 13. Suggested PR breakdown

1. **`feat(ui): TurnPresence model + formatPresence`**  
2. **`refactor(ui): demote secondary status surfaces`**  
3. **`style(ui): bomb mood retune + enter transitions + honest meter`**  
4. **`feat(ui): stage honesty + tool chips + stall kinds`**  
5. **`feat(ui): per-session presence map`**

Each PR should include a short screen recording of one happy-path turn.

---

## 14. Appendix A — Current phase/mood map (reference)

```
PHASE_LABEL: idle Idle | send Sent | think Thinking | tools Using tools
             | reply Writing reply | wait Needs you | done Done | error Failed

PHASE_MOOD:  send/think → thinking
             tools → tooling
             reply → stream
             wait → wait
             done → boom
             error → error
             stall override → running  (REMOVE in Phase C)
```

## 15. Appendix B — File ownership

| Concern | File |
|---------|------|
| Presence logic | `frontend/app.js` → prefer `frontend/presence.js` |
| Bomb motion | `frontend/bombs.css` |
| Dock/composer layout | `frontend/bombs.css` + `frontend/styles.css` |
| Markup hooks | `frontend/index.html` |
| This plan | `docs/plan/status_and_bomb_animation_ux_plan.md` |

---

## 16. Next action

When ready to implement: start **Phase A** (presence model + single formatter) without visual CSS changes, then **Phase B** demotion—biggest perceived calm-down for least art risk—then **Phase C** animation retune.

**Do not** retune CSS first while five surfaces still shout the same phase.

---

## 17. Implementation log (2026-07-11)

Multi-agent loop: **implement → audit (code-reviewer + silent-failure-hunter) → revise → retest**.

### Shipped
| Phase | Deliverable |
|-------|-------------|
| A | `frontend/presence.js` + `formatPresence` / stall / sticky tools; `app.js` consumes it |
| B | Host-only status pill; satellite bombs; timeline bombs only on milestones |
| C | Mood retune (no stall shake), `is-entering`, honest meter modes, flavor after 8s |
| D lite | `stagesSeen`, stall kinds, tool sticky, wait stage honesty |
| E lite | `presenceBySession` for **all** session events; session-scoped boom timer |

### Audit fixes applied
1. Tool **id set** + terminal statuses (`failed`/`denied`/…) so `toolsActive` cannot stick  
2. Session-scoped **boomTimer**  
3. Presence updates for **non-selected** sessions  
4. `startAcp` / `refreshSessions` go through `selectSession`  
5. Agent errors no longer overwrite host Ready pill  
6. `aria-live` on phase title only  
7. Guard if `presence.js` fails to load  
8. `frontend/presence.test.mjs` — run with `node frontend/presence.test.mjs`

### Files
- `frontend/presence.js` (new)
- `frontend/presence.test.mjs` (new)
- `frontend/app.js` (presence integration)
- `frontend/bombs.css` (motion hierarchy)
- `frontend/index.html` (script order, dock a11y)
