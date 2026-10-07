# See Cubed release qualification

Requested by Paul on 2026-10-07. Baseline: `baa890860d7f6b703757e62f9b58b33a7fe82648` from `jedisherpa/grok-build-tauri-control-panel`, branch `main`. Work lane: `codex/c3-release-qualification` in the fresh SE clone. Existing primary, observatory, design and GPUI worktrees remain separate.

## Objective

Qualify the existing Rust/Tauri product through feature inventory, explicit user stories, repeated implementation tests and actual packaged-app workflows. Repair defects and revise inaccurate stories with a recorded reason; never revise expectations merely to make failures pass. Publish the exact qualified source and distribution artifact, then obtain Apple notarization and verify the stapled download through Gatekeeper.

## Ordered work

1. PM: inventory user-facing features, native commands, persistence/authority boundaries, existing tests and runtime prerequisites. Establish stable story IDs and honest evidence states.
2. Implementation: implement release preparation/verification tooling and fix defects found by stories. Retain failures, data hashes and evidence outside Git. Use generated test conversations/projects for live provider tests; do not transmit private archives for QA.
3. Architecture refinement: independently inspect lifecycle, authority, persistence, packaging and story/evidence completeness; repair findings and repeat affected tests.
4. Performance/simplicity refinement: inspect resource bounds, animation/accessibility, startup/scrolling, distribution dependencies and operational complexity; repair findings and repeat affected tests.
5. PM validation: full Rust workspace check/test/strict Clippy, complete frontend/Python tests, native packaged workflows, deliberate interruption/restart, failure scenarios and preserved-state checks. Re-run the final suite after relevant changes.
6. Release: pin clean commit, build declared macOS architecture(s), sign with Developer ID and hardened runtime, test that signed candidate, notarize, staple, verify signature/ticket/Gatekeeper and exact artifact digest, then publish approved production assets.

## Gates

- A passing unit or mocked UI test does not satisfy a native, real-provider, clean-machine, performance, accessibility, signing or distribution story.
- Evidence states are `not_run`, `passed`, `failed`, `blocked`, or `not_applicable` with a reason. A feature remains unqualified until its required levels pass.
- Changing code invalidates affected evidence. Test records identify source revision and artifact hash; unstaged source is identified by a patch digest.
- Preserve user config, archives, sessions, notes, indexed memory, Joe receipts and worktrees. Test mutation stays in explicitly generated fixtures or an isolated QA profile.
- Preview starts no agent; plan approval precedes editing; result acceptance precedes dependent execution. Coordinates/activity never grant authority or establish semantic truth.
- Production publication follows successful qualification, not merely successful compilation. Open gates are reported explicitly.

## Initial prerequisites

Mac: Apple Silicon, macOS 15.6.1, Xcode 26.3, Rust 1.93.1. `notarytool` is present. `security find-identity -v -p codesigning` reports zero valid identities. Apple Developer ID setup and desired distribution architectures are pending user clarification; unrelated qualification continues.

## Primary references

- https://v2.tauri.app/distribute/sign/macos/
- https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- https://developer.apple.com/documentation/security/customizing-the-notarization-workflow

Private raw outputs and machine-specific test data belong in the sibling SE `outputs/c3-release-qualification` directory, not in the repository.
