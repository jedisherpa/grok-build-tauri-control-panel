# See Cubed macOS distribution

This procedure qualifies the existing Rust/Tauri product. Signing and notarization are separate from the feature stories in `USER_STORIES.md`: Apple's automated scan does not establish that the app works correctly. No command below has been executed merely by writing this document.

## Developer builds and safe installation

`scripts/install.sh` now requires an explicit developer invocation and a **new** destination. It refuses an existing app, including a symlink, verifies the built signature, copies it without re-signing, verifies the copy, and leaves launching to the operator. It no longer deletes `/Applications/Bomb Code.app`, replaces a Developer ID signature with an ad-hoc one, swallows signature errors, or launches into the user's normal profile automatically.

```sh
scripts/install.sh --development --destination '/absolute/new/Bomb Code QA.app'
```

The destination parent must exist. Normal build output is `target/release/bundle/macos/Bomb Code.app`; `CARGO_TARGET_DIR` is respected. The installer rejects notarization-related environment variables because Tauri can automatically submit a build when those credentials are present. It does not print their values. An interrupted copy can leave a partial **new** destination; preserve it for diagnosis and choose a different destination for the retry.

For an upgrade, keep the previous app and its digest in a rollback directory, stop it, and install the qualified replacement under a new name before reviewing the replacement of the normal application path. This developer script deliberately does not automate that replacement. Never re-sign a tested Developer ID candidate after copying it.

## Read-only artifact receipts

Run this before native QA, after Developer ID signing, and after stapling. Each invocation requires a new receipt directory outside the Git source, app bundle and Applications directories.

```sh
python3 -B scripts/release_preflight.py \
  --repo "$PWD" --app '/absolute/Bomb Code.app' \
  --output '/absolute/evidence/qa-01' --stage qa --architecture arm64
```

The manifest records the full HEAD, branch, staged and unstaged binary patch digest, untracked source hashes, bundle identifier/version, executable digest, bundle contents/modes/symlinks/extended-attribute hashes, tool commands and results. `source.patch` preserves the tracked difference. Receipts can contain source changes and local paths: keep the evidence directory private; never commit raw receipts or private QA data.

QA permits dirty source and valid ad-hoc signatures. `--stage candidate` additionally requires clean source, Developer ID Application authority, a Team ID, secure timestamp, hardened runtime, readable entitlements without debug access, matching signature identifier, the exact declared CPU architectures and macOS SDK ≥ 10.9. Every nested Mach-O image must also have Developer ID authority and a secure timestamp; deep/strict bundle verification checks signature validity. `--stage distribution` also requires stapler validation and Gatekeeper assessment. Failed checks still produce a receipt and exit 1; malformed/changing inputs or unsafe output paths exit 2. Tool failures fail closed.

For a universal app, declare both `--architecture arm64 --architecture x86_64`. A single-architecture result does not qualify another architecture. The manifest explicitly says `product_qualification: not_asserted` and `artifact_source_provenance: not_asserted`: a separately preserved build log/receipt must connect the pinned source, toolchain, lockfile, build flags and output, and native stories must identify that output's digest. Signing or stapling changes the artifact; issue a new receipt rather than reuse old evidence.

## Developer ID candidate

First resolve all required stories and failures, commit the qualified source, and select the distribution architectures. Check `security find-identity -v -p codesigning` locally. Outside-App-Store distribution needs a valid **Developer ID Application** identity, not an Apple Development or ad-hoc identity. This Mac initially had zero valid identities, so no signed release was available at qualification start.

Build with the selected identity and hardened runtime, with notarization credentials absent so build and submission remain separate reviewed steps. Example for Apple Silicon, after identity setup:

```sh
env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
  -u APPLE_API_KEY -u APPLE_API_KEY_PATH -u APPLE_API_ISSUER \
  APPLE_SIGNING_IDENTITY='Developer ID Application: YOUR NAME (TEAMID)' \
  cargo tauri build --target aarch64-apple-darwin --bundles app \
  --config '{"bundle":{"macOS":{"hardenedRuntime":true}}}' -- --locked
```

Preserve the build log, compiler/Xcode/Tauri versions, target, lockfile digest and source pin. Run candidate preflight against the resulting target-specific bundle. Test **that signed candidate** under an isolated QA profile: restart/recovery, CLI launching, reviewed workflows, memory and MCP behavior, permissions and denied paths, access to required files, reduced motion, and other required stories. Hardened runtime can change behavior, so unsigned QA does not replace this pass. External CLI dependencies must be documented and tested; this app does not embed or authenticate Grok/Codex/Claude merely by being signed.

## Notarization, stapling and final download

Only submit after the signed candidate's required stories pass. Create a keychain profile interactively with `xcrun notarytool store-credentials 'c3-notary'`. The tool prompts for credentials, and its default validation contacts Apple; do this only when that account setup is intended. Never put passwords or exported private keys in Git, scripts, command-line history or QA output.

Create a new private distribution directory, ZIP the signed app with Apple's `ditto` method, and submit that archive:

```sh
mkdir '/absolute/new-distribution'
ditto -c -k --keepParent '/absolute/Bomb Code.app' '/absolute/new-distribution/submission.zip'
xcrun notarytool submit '/absolute/new-distribution/submission.zip' \
  --keychain-profile 'c3-notary' --wait --output-format json
```

Preserve the submitted ZIP digest and returned submission ID/status. Require **Accepted**, retrieve and review `notarytool log` even on success, and retain warnings. Notarization alone does not qualify production. Staple the ticket to the app, validate it, and run distribution preflight:

```sh
xcrun stapler staple '/absolute/Bomb Code.app'
xcrun stapler validate '/absolute/Bomb Code.app'
python3 -B scripts/release_preflight.py \
  --repo "$PWD" --app '/absolute/Bomb Code.app' \
  --output '/absolute/evidence/distribution-01' --stage distribution --architecture arm64
ditto -c -k --keepParent '/absolute/Bomb Code.app' '/absolute/new-distribution/See-Cubed-macos-arm64.zip'
shasum -a 256 '/absolute/new-distribution/See-Cubed-macos-arm64.zip'
```

A ZIP cannot itself be stapled; rebuild the final ZIP from the stapled app. Treat submission and final ZIP as distinct digests. If distributing a DMG, verify its integrity and assess/staple that actual container too; this app-only preflight does not qualify a DMG. Test the final downloadable, quarantined artifact on a clean account/Mac with required external CLIs installed and unavailable. Preserve signature, ticket, Gatekeeper, launch, recovery and installed-data-preservation evidence. Publish only the final tested asset with the matching clean source tag and digest; verify the downloaded asset again. Retain the preceding app for rollback without discarding its settings/history.

## References checked on 2026-10-07

- [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/): identity configuration, separate ad-hoc behavior, and automatic notarization credentials.
- [Apple notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution): Developer ID, hardened runtime, timestamp and entitlement requirements.
- [Apple custom notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow): `notarytool`, submission/log preservation, `ditto`, stapling and ZIP rebuilding.

The installed `cargo tauri build --help` and `xcrun notarytool store-credentials --help` were read to check supported CLI syntax. This document is preparation; actual signing, submission and downloaded-runtime acceptance remain separate evidence gates.
