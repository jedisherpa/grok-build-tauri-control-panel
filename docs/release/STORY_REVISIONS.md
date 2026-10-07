# Story revisions and defects

## Initial discoveries (2026-10-07)

- C3-039: prior local installation worked because Joe/dictionary paths named `/Users/paulcooper` and a particular NVM Node version. Revised expectation: resolve the current user's host-owned frozen package and Node from executable PATH without accepting UI-supplied runtime paths. Local source is being repaired; availability of the frozen reference/Python/model on a clean Mac remains an independent gate.
- C3-043: prior installer deleted the destination and force re-signed it ad-hoc while swallowing signing errors. Expected release behavior requires retaining the tested Developer ID signature and a safe, reviewable install/rollback path. Implementation is being repaired.
- C3-018: preview is implemented for the first part of a numbered sequence. The story explicitly exposes this limitation, rather than claiming all parts were previewed. Later actual submission still validates each part in the native host; partial submission is independently recorded in C3-019.
- C3-040: current UI says See Cubed while package name/docs remain Bomb Code. Complete visible branding with unchanged storage/bundle identity where required for migration.

Future revisions must name story ID, original expected behavior, observed evidence, whether this is a defect or an expectation correction, exact change and retest result. A passing smoke check never replaces an unrun required story.

## Repair and audit checkpoint

- C3-002/039: QA path selection alone is insufficient isolation because vendor CLI authority and WebKit localStorage remain separate. Main MCP mirroring is disabled and Haven config now routes to the profile; use fake adapters or a separately isolated account/browser store before native mutation tests.
- C3-032: observed source changed punctuation-only search to browse-all while the unchanged literal-search test expected zero. Restore zero matches for a nonempty query without searchable tokens; empty query still browses. Initial failing receipt retained; complete Python retest passes.
- C3-036: corrupt startup configuration is preserved; named regression covers corrupt base and corrupt overlay before discovery save. Native recovery/error UX still open.
- C3-025: generated concurrent notes exposed shared-temp publication races. Named regressions now verify 32 simultaneous adds and exact reopen IDs, failed-add visibility, and preservation of corrupt source/prior backup; multi-process and directory/power-loss durability still open.
- C3-012/038: byte clipping now retains UTF-8 boundaries at terminal/file caps; named regression passes, but whole-file allocation and admission bounds remain open.
- C3-004: paused/reduced-motion sheets still do work; platonic solid auto-rotation does not honor application Pause/hidden lifecycle. Keep this as an implementation defect; do not redefine Pause to match it.
