# Native policy broker: preserving implementation plan

Drafted 2026-10-07 for independent architecture review. This document authorizes
no provider calls, authentication changes, adapter installation, app replacement
or production qualification. It supplements
`release_authority_execution_events.md`; its ordered gates remain authoritative.

## Goal and current boundary

Restore default Plan and reviewed Build roles while preserving native Grok,
Claude and Codex conversations, authentication, resume identity and model
behavior. Retain C3's memory/citations, E8, SenseSnap, CDISS and Joe contracts.
There is one workflow runner, one workspace admission coordinator and one
deny-first evaluator. The broker is an additional transport into existing host
dispatch, not another execution plane.

Current ACP launch rejects native Plan, read-only, Workspace/Strict and explicit
deny policies because internal tools may bypass host callbacks. This is an
honest intermediate guard, not acceptable final feature completion. Explicitly
unrestricted ordinary native sessions remain separately identified. Do not remove
the guard based on advertised modes, successful initialization or a prompt that
asks a model to behave.

## Evidence and qualification limits

Inspected local CLI help/version and installed adapter source read-only. No
agent was prompted, authentication/configuration rewritten, private history read,
or installed app changed. Hashes below identify observed research inputs; they
are not certified runtime pins or final release receipts.

| Observed input | Identity / SHA-256 |
|---|---|
| Grok executable `/Users/paulcooper/.grok/bin/grok` | `1.0.46 (2765805b9442) [stable]`; `e8daa302364c9c3b6a5546d511cfbd1ab5e5d407a9b04282f660665ea405f9f3` |
| Installed Claude ACP package | `@agentclientprotocol/claude-agent-acp` 0.49.0; SDK 0.3.185 |
| Claude `dist/acp-agent.js` | `ebcb132530529a76cb4b46b44b77833bf6706e069855058c5e08d40445759f4f` |
| Installed Codex ACP package | `@agentclientprotocol/codex-acp` 2.1.1 |
| Codex `dist/index.js` | `4b76310393d756a0f111687cd9df899720f36f7c59b1eb1d86429484034ff91b` |

Both adapter packages were read under
`/Users/paulcooper/.grok/control-panel/adapters/node_modules/`.
Single source-file hashes do not cover transitive dependencies or bundled native
binaries. Actual certification must include those artifacts.

Grok public source inspected at the time reported
`SOURCE_REV=559751fdcec02d413e4c57c8832ab275e4f44980`. That revision is different
from the installed executable's reported revision. Public source demonstrates
design constraints, not that the installed binary has identical behavior. The
public URLs below track upstream `main`; archive the exact reviewed bytes and
their hashes in implementation receipts rather than treating mutable URLs as pins.

### Claude

Installed `acp-agent.js` accepts `_meta.claudeCode.options`; lines 2320–2323 use
`tools: []` or the legacy `disableBuiltInTools` flag to suppress built-ins.
Lines 2331–2333 allow overriding `settingSources` with an empty array. Supplied
MCP servers are merged at 2355. These are concrete prototype controls.

However, `SettingsManager` initializes at 2251 before those metadata options are
read. Its user/project/local settings are used to select native permission mode
at 2299. Native mode is mutable, bypass mode can allow tools without an ACP
approval, and user options can introduce hooks, additional directories and MCP.
Therefore metadata overrides alone do not prove whole-runtime containment.

Use a bundled, pinned adapter patch that constructs host-approved options before
settings initialization: built-ins empty, approved settings sources only,
external hooks/plugins/subagent discovery disabled and exactly one trusted MCP
bridge. Preserve native query/resume and vendor-managed restrictions; host
policy may tighten, never relax an effective managed restriction. Inspect SDK
behavior with a fake SDK before deciding which options alone are sufficient.

Claude's native sandbox primarily covers Bash. File tools, MCP and hooks need
separate controls. CLI `--bare` changes authentication behavior, so it must not
be substituted casually for a preserving authenticated runtime.
[Official sandbox documentation](https://code.claude.com/docs/en/sandboxing),
[official CLI reference](https://code.claude.com/docs/en/cli-reference),
[adapter source](https://github.com/agentclientprotocol/claude-agent-acp).

### Grok

Installed help advertises tool masks, disallowed tools, deny rules and subagent
controls. Do not infer that all top-level flags reach `agent stdio`.
Inspected public `xai-grok-pager-bin/src/main.rs` lines 2228–2248 forward only
permission mode, trust, update and web-search options to `run_agent_command`;
top-level tool masks are not forwarded there. Runtime resolution at 1343–1355
supplies `cli_subagents: None`. `xai-grok-pager/src/agent_runtime.rs` then delegates
stdio to the shell runtime using that configuration. Verify the installed binary
separately before relying on any flag.

`GROK_CONFIG` / `GROK_CONFIG_PATH` overlays are restricted to a soft-settings
allowlist. They are not arbitrary security-policy injection. `GROK_HOME` is a
supported vendor state location, but relocating it alone can lose authentication
or native resume state; design a verified read-only auth/state handoff before use.

Grok's whole-agent confinement profiles are a useful second boundary. Read-only
still allows writes to vendor runtime state/temp; it does not prove tool-level
deny enforcement. Existing leader processes cannot be treated as confined merely
because the new client is confined: inspected upstream leader code rejects
non-off confinement before connecting/spawning. A certified launch must use a
dedicated direct process and prohibit existing leader sockets. Documented child
network denial is a no-op on macOS; no arbitrary external MCP can be admitted
as read-only based on that profile.

Prototype a pinned direct-stdio runtime with an effective toolset restricted to
the host bridge, no automatic MCP/plugin/subagent discovery and no automatic
updates. If current binary controls cannot establish this, keep the guard and
review a pinned runtime patch; do not substitute an API-only chat implementation.
[Entrypoint source](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager-bin/src/main.rs),
[stdio runtime](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/src/agent_runtime.rs),
[leader confinement test](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/tests/test_leader_sandbox_confinement.rs),
[official sandbox documentation](https://docs.x.ai/build/features/sandbox),
[official settings documentation](https://docs.x.ai/build/settings).

### Codex

Installed adapter supports `CODEX_CONFIG`, `CODEX_PATH` and `INITIAL_AGENT_MODE`.
Its `sendPrompt` passes native approval and sandbox policy every turn
(`index.js:34350`). Native ReadOnly includes network disabled; WorkspaceWrite
is distinct. Additional directories expand writable roots, and mode changes can
select wider policies. Starting read-only is not an immutable host ceiling.
`createSessionConfig` marks project roots trusted; an empty supplied MCP list
does not itself establish that configured MCP discovery was removed.

Official `features.shell_tool=false` / `features.unified_exec=false` switches
control shell execution. They do not prove suppression of every built-in tool,
including patch/file effects. No verified disable-all-built-ins control was found
in the inspected inputs. This is an unresolved capability, not a claim that no
such control can exist.

App-server experimental `dynamicTools` can expose host-owned tools and persist
them on resume, but the current ACP adapter does not forward them. A pinned
adapter fork could bridge them while preserving app-server conversation identity.
First determine whether the native runtime can suppress all competing effect
tools. Outer filesystem denial alone may prevent writes but leave advertised
tools repeatedly failing; that is not a finished product flow.
[Adapter repository](https://github.com/agentclientprotocol/codex-acp),
[official configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference),
[official app-server documentation](https://learn.chatgpt.com/docs/app-server).

## Proposed interfaces and ownership

These are design contracts, not implemented APIs:

```text
RuntimePolicy {
  immutable_ceiling, canonical_workspace_lease, deny_rules,
  approved_broker_manifest_digest
}
RuntimeCapabilityReceipt {
  backend_executable_digest, adapter_tree_digest, sdk_native_digest,
  version_identifiers, accepted_policy_schema,
  tool_suppression_probe_digest, config_source_probe_digest,
  outer_confinement_probe_digest
}
NativeLaunchPlan {
  exact_argv, permitted_environment, private_runtime_directories,
  trusted_bridge_binding, effective_config_digest, capability_receipt
}
BrokerDispatch(request_id, session_generation, epoch, HostAction)
```

The launch planner lives at the existing backend resolution/ACP spawn boundary.
Only an accepted capability receipt may clear `native_runner_unconfined` for
its exact policy and artifact set. Reject unknown artifacts before launch;
certified paths cannot use unpinned `npx --yes` fallback. Resolve/canonicalize the
executable, interpreter and dependency tree; check again before execution so a
PATH change cannot silently replace the certified backend.

The native process has a fixed outer ceiling: no direct workspace writes and
only required private runtime/cache writes. Authentication and resume state are
preserved through a reviewed vendor-specific mechanism; no broad writable grant
to the user's existing `.grok`, `.claude` or `.codex` stores. Do not repurpose the
machine's `HOME`/`CODEX_HOME` shell variables to route arbitrary test state.
Test and validate supported vendor-specific paths without changing real auth.
Whole-process confinement must cover spawned children and startup hooks. Prove
that the exact final signed runtime can establish it; sandbox initialization
failure is a launch failure.

The actual broker runs in C3's host, outside the native sandbox. An MCP bridge
inside the sandbox talks only to that host-owned endpoint. The endpoint binding
is allocated by the host, scoped to a session generation and is not a UI-supplied
owner ID. Transport credentials must not become model-visible authority.

Typed broker read/write/process actions use existing `HostAction` classification,
canonical path checks, deny-first evaluator, one-use pending grants, cancellation
epoch and workspace lease. Validate at park, human response and dispatch; revoke
on cancellation/shutdown. Unknown actions fail closed. Native Plan/Ask/Yolo may
change presentation and host approvals within the immutable ceiling. A read-only
role can never gain a write/process grant through a mode change or native resume.

Keep transport lengths/timeouts bounded. Process actions use the existing terminal
and process supervisors; do not infer completion from output or idle state.
Recovery and durable event semantics remain the ordered Gate 2/3 work, and a
broker operation cannot introduce automatic replay of interrupted effects.

## Prototype order and exact adversarial checks

All initial fixtures use generated workspaces, fake adapters/SDKs and isolated
stores. They must not read original archives, load real credentials, call a
provider or mutate installed adapters. Preserve failing receipts.

1. **Claude option-capture prototype.** A fake SDK captures the options received
   before query startup. Assert built-ins empty, approved settings sources only,
   one bridge MCP, no extra directories and no user hooks/plugins/subagent source.
   Plant generated user/project settings that request bypass, a hook and external
   MCP; assert none starts and none widens the ceiling. Verify managed restrictions
   are retained or incompatibility rejects launch.
2. **Grok direct-stdio propagation probe.** Use a pinned runtime's offline test
   seam to inspect its resolved toolset, subagent discovery and leader decision.
   Assert no internal effect tool and no existing leader connection. A fixture
   that drops CLI masks must fail certification. Help output and `initialize`
   success cannot satisfy this check. If no provider-free inspection seam exists,
   implement/review one in the pinned runtime before claiming the capability.
3. **Codex app-server capture prototype.** Fake app-server records thread start,
   resume and every turn. Assert the fixed native ceiling, no widening additional
   directories, no configured external MCP and only the approved dynamic tools.
   Exercise every mode RPC and persisted wider prior mode. Require separate
   evidence that builtin patch/file/process tools cannot execute; do not mark
   certification passed merely because shell feature flags were emitted.
4. **Outer confinement fixtures.** A generated child attempts workspace/outside
   writes, canonical aliases, hardlinks, nested descendants, hook execution,
   shared leader socket connection and runtime-path substitution. Sentinel files
   must remain unchanged; only designated private runtime writes succeed. Test
   unavailable confinement and poisoned runtime paths reject before launch.
5. **Broker authority fixtures.** Deny a write in immutable ReadOnly; reject a
   forged session binding, mismatched action hash, stale epoch, repeated grant ID,
   canceled approval and Plan narrowing racing dispatch. Explicit deny beats
   Ask/Yolo and persistent/session allow rules. A legitimate reviewed write uses
   the canonical lease and affects only its approved generated path.
6. **Pin and continuity fixtures.** Alter executable/adapter/SDK bytes and assert
   rejection before startup. Unknown version/config keys cannot fall back. Resume
   the same generated native conversation while retaining the host ceiling;
   a stored unrestricted native mode or restored tool manifest must not widen it.
7. **Integrated workflow and final package.** After source checks, run a complete
   generated conversation → cited recall → reviewed change → explicit commit →
   Land/Sync story. Test broker shutdown, restart and pending operations under the
   Gate 2/3 recovery model. Repeat confinement/descendant checks on the exact signed
   hardened runtime candidate, then packaged accessibility/performance/recovery
   checks before notarization. Signing is not a functional test receipt.

After these offline checks, any required live native/provider smoke must use an
explicitly authorized generated payload/destination, retain native identity and
produce an exact artifact receipt. Private-history tests require their own scoped
authorization and cannot be inferred from approval of generated fixtures.

## Acceptance decision

Architecture review must resolve Codex builtin suppression, Grok stdio propagation,
Claude pre-start configuration sources, auth/resume preservation and macOS outer
confinement before implementation can claim complete coverage. Enable backends
only for capabilities proven by their own pinned receipts. Intermediate gaps stay
visible and fail closed; default Plan and reviewed Builds across intended release
backends are required product behavior, not optional regressions to hide.

The parent owns implementation dispatch, code review, full source gates, release
checkpoint and final signed qualification. This research document alone changes
no runtime authority and closes no production gate.
