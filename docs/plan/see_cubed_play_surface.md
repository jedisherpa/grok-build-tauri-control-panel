# See Cubed play surface

**Status:** Implementation contract for the Tauri app.  
**Date:** 7 October 2026  
**Lane:** `/Users/paulcooper/Documents/Codex/2026-10-06/tak/work/bomb-code-installed-rebuild` on `codex/installed-plus-github-tauri` at `b4ede97`.  
**Do not touch:** `see-cubed-design`, the GPUI tree, prove-pack evidence, or `bomb-code-primary`. Do not push.

## What this is

The eight interaction rules are design guidance for this product, not a second app. The observatory review at `tak/outputs/bomb-code-observatory/review.html` (source `2ec02d6`) is already in this line: midnight glass, static lattice guides, independent cubes, and the composer on the working surface. Keep that pass. Do not restyle it and do not touch saved sessions.

The play surface is the opening of the existing spatial studio. Home leaves play and restores the desk, including `#composer`.

## Rules on this surface

1. Launch opens the already-moving E8. The first hit does the session verb. The five-choice invitation is not the opening.
2. One verb per entry: `spin`, `stretch`, `drop`, `paint`, or `hum`. Default `spin`. The kit that changes the verb sits on the desk, and the play surface only does the stored verb.
3. A loop lasts 10 seconds and carries no score, combo, streak, or unlock. The next loop does not read the previous one.
4. A hit is a large slow response. A miss is a different animation. There is no fail copy, timer, or streak.
5. Each hit and miss has a generated tone. Silence is one thumb-sized control. No lyrics and no audio files.
6. The form is a palm-sized target. Corners have no controls. Tap, a 600ms dwell, and a mash do the same verb. The only word on the surface is the verb, huge and still. It is also spoken unless silenced.
7. `c3:session-skin:v1` may be empty or a name set on the desk. Play does not ask, and the skin does not change the verb, the word, or the tone.
8. Stopping leaves the E8 idling. Coming back is the same surface. Play cannot send, land, post, buy, message, consent, or change a dose. Help (tour, Joe, settings, composer) is reached by leaving play.

## Files

- Add `frontend/studio-play.js`, `frontend/studio-play.css`, and `frontend/studio-play.test.mjs`.
- Link the stylesheet and script from `frontend/index.html` beside the other studio files.
- From `studio-play`, add `is-studio-play` on the document element, hide `#composer` and the invitation, and listen for the E8. `#studio-home` removes `is-studio-play` and shows `#composer` again.
- Do not change E8 projection math in `studio-motion.js`.

## Tests

`node --test studio-play.test.mjs studio-entry.test.mjs studio-motion.test.mjs studio-tour.test.mjs` from `frontend/`. The pure API covers verb fallback, 10-second loops, miss versus hit, skin independence, tones, silence, and the absence of consequence commands.

## Build

Reuse `CARGO_TARGET_DIR=/Users/paulcooper/Documents/Codex/2026-10-06/tak/work/bomb-code-observatory/target`. Do not clean or delete that target. About 11 GB is free. After tests pass, build the Tauri bundle and install it over `/Applications/Bomb Code.app` only by quitting that app first. Do not commit and do not push.
