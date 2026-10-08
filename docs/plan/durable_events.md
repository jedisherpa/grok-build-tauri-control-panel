# Durable events and recovery implementation contract

Implement after the process-completion source gate. Preserve the existing event
types, sessions, transcript IDs, notes, history, interpreted profiles and source
receipts. This is one ingress/projection owner for the current product; it adds
neither another workflow runner nor another memory database.

## Admission and runtime identity

Acquire ownership of the physical profile/store before configuration, memory,
scheduler or workflow mutation. A process-lifetime OS file lock must resolve
canonical aliases and refuse a second writer without breaking a supposedly
stale lock. Another legacy installed app that does not participate in this lock
must be closed before normal-store activation; a new lock cannot control it.

Upgrade the locked bundled Rust SQLite runtime from the observed 3.46.0 input.
The cached rusqlite 0.40.2 / libsqlite3-sys 0.38.2 input contains SQLite 3.53.2,
source ID `2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24`.
Verify these values from the linked runtime, not merely the manifest. Read back
WAL, synchronous FULL, foreign-key and busy-timeout settings and fail visibly if
the effective settings differ. SQLite documents the repaired WAL-reset race and
the durability difference between NORMAL and FULL in its [WAL documentation](https://www.sqlite.org/wal.html).
This version choice is a locally inspected dependency input, not a claim that it
is the newest release or free of all defects.

## Commit before notification

Install an injected durable sink in EventBus before any producer starts.
Serialize publication through one owner. A single SQLite transaction appends the
event and updates transcript/status projections. Only its successful commit may
publish the committed envelope: store ID, generation, sequence and event.
Remove the independently lagging persistence subscriber in the Tauri setup.

Store generation alone does not fence a resumed native runner. Assign an opaque
host runtime generation before each session producer starts and carry it through
committed envelopes, status changes and turn completion. Scoped producer handles
share the same publication owner and storage sink; they are not separate buses
or runners. Registry/projection consumers must reject an older runtime's status
after a same-ID resume, and reject a retired turn's approval/completion. Caller
or provider text cannot choose or forge those generations. A backlog of an old
Cancelled event cannot stop a newly resumed conversation.

Keep notification subscribers inexpensive; loss of a subscriber cannot lose a
committed event. Critical admission, dispatch, approval and completion paths use
checked publication. Persistence failure must reject subsequent consequential
work and expose unhealthy coverage, rather than emitting success with missing
evidence. The sink must never recursively emit, take service locks or call back
into workflow/registry owners. Do not use an unacknowledged queue as a durability
substitute. Measure synchronous commit backpressure before choosing a more
complex acknowledged writer actor.

Persist user submission and operation intent before transmission/effect where
the existing interface currently writes rows independently. Keep source message
boundaries and existing flattened transcript behavior distinct; do not duplicate
AgentOutput text already projected through AgentMessage. Historical permissions
are inert. Removal tombstones prevent late events recreating deleted sessions.

## Bounded reconciliation

Expose typed snapshot/replay pages with explicit store/generation identity,
watermark, cursor, limits and coverage. Subscribe before fetching a snapshot;
buffer arriving notifications, apply the snapshot, then deduplicate/replay events
above its watermark. On lag, reconnect or generation change, reconcile rather
than silently keeping an incomplete UI. A snapshot and its watermark must come
from one consistent read transaction. Avoid comparing transcript lengths as a
proxy for completeness or marking a failed load permanently successful.

## Required generated evidence

- Producers with zero subscribers still commit exact events/projections.
- A slow or dropped UI subscriber reconstructs ordered interleaved text, tools,
  approvals and terminal output without duplicates.
- Commit failure rolls back both event and projection, exposes health failure,
  and prevents a new spawn/grant/effect from being presented as successful.
- Restart between intent, effect and outcome preserves uncertain recovery;
  restored approvals cannot authorize new requests.
- Canonical profile aliases and a second process cannot acquire concurrent
  writer ownership; lock release requires the actual owning process exit.
- Removal, replay-page boundaries and stale generation/cursor cases cannot
  resurrect sessions or drop committed text.
- WAL-aware backup/reopen/checkpoint preserves committed data and runtime
  settings. A raw main-database copy is not sufficient recovery evidence.
- Bounded stream/commit latency, runtime-worker stalls and renderer reconciliation
  are measured again on the exact final signed artifact.

Keep pre-fix failures and raw receipts outside Git. Passing source tests does not
qualify disk power-loss behavior, legacy app exclusion, a native package or a
production download.

## Implemented source contract and explicit limits

The retained SQLite writer commits journal, transcript/status projection, managed
session metadata, workflow/scheduler snapshots and correlated operations in the
same transaction before notification. Production keeps physical profile and
main/WAL/SHM identity ownership through every persistence clone. Identity checks
run before transactions and after commit before acknowledgement; a post-commit
identity failure preserves the old receipt and reports uncertainty. It cannot
assert rollback of an already committed external effect.

Snapshot leases pin a consistent read transaction for bounded pages. Leases have
count/time limits; replay and snapshots have row and byte limits. The renderer
keeps 2,000 rows/4MiB and at most 300 transcript DOM rows with explicit earlier
coverage. Structured native messages that cannot fit the accepted canonical
projection fail current-runtime coverage, rather than disappearing before a
later successful completion. Historical approval text remains inert; restored
controls come only from bounded current-host pending requests carrying exact
runtime and turn identity.

A prior process lifetime is retired on startup. Nonterminal records become
recovering, unresolved operations retain their private IDs, and no effect is
replayed automatically. A released store lock proves prior writer exit, not
native child cleanup or external effect completion. Consistent backups preserve
recovery identity; concurrently activating a separately cloned profile still
requires an explicit generation-rotation protocol. Legacy process exclusion is
a bounded known-product observation, not control of unknown noncooperating apps.

Synchronous FULL commits remain subject to final signed-app responsiveness
qualification. The generated owned-writer probe and deliberately non-yielding
burst are useful source measurements, not native frame-time or recovery results.
Untagged provider messages actually emitted after a new turn starts cannot prove
their causal turn. Dedicated process groups do not contain detached descendants.
Other utility workers, full native tool enforcement and aggregate admission are
subsequent release gates. Do not publish these source results as packaged or
power-loss qualification.
