#!/usr/bin/env python3
"""Check pinned packaged evidence before notarization; never signs or submits.

This checks receipt consistency and completeness, not the truth of observations
or human release approval. Keep input receipts and generated QA data private.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import plistlib
import re
import sys

from release_preflight import artifact_snapshot, file_digest, source_snapshot, inspect as inspect_candidate


class EvidenceError(ValueError):
    pass


CASES = {
    "workflow": {"operation-policy", "exact-approval", "workspace-ownership", "reviewed-change"},
    "recovery": {"process-cleanup", "scheduler-interruption", "event-reconciliation", "inert-recovery"},
    "accessibility": {"keyboard", "voiceover-motion"},
    "performance": {"matched-scenarios"},
}
SHA = re.compile(r"[a-f0-9]{64}\Z")
REQUIRED_LEVELS = {f"C3-{i:03}": {"packaged"} for i in range(1, 43)}
for number in (9, 10, 11, 29):
    REQUIRED_LEVELS[f"C3-{number:03}"] |= {"provider"}
for number in (2, 20, 34, 38):
    REQUIRED_LEVELS[f"C3-{number:03}"] |= {"recovery"}
for number in (37, 39):
    REQUIRED_LEVELS[f"C3-{number:03}"] |= {"external"}
REQUIRED_METRICS = {"cold-startup-ms", "warm-startup-ms", "input-ms", "scroll-ms",
                    "native-hitch-ms-per-s", "paused-cpu-percent", "shell-rss-mib",
                    "webkit-rss-mib", "provider-rss-mib", "navigation-growth-mib"}
METRIC_UNITS = {name: ("ms/s" if name == "native-hitch-ms-per-s" else
    "%" if name == "paused-cpu-percent" else "MiB" if name.endswith("mib") else "ms")
    for name in REQUIRED_METRICS}
CANDIDATE_CHECKS = {"bundle_identifier", "bundle_version", "signature_valid", "declared_architectures",
    "clean_source", "signature_metadata_available", "signature_identifier", "entitlements_readable",
    "developer_id", "team_identifier", "secure_timestamp", "hardened_runtime", "no_debug_entitlement", "supported_sdk"}
NATIVE_CAPABILITIES = {"plan", "immutable-read-only", "workspace-containment", "deny-first", "children-contained"}


def require(condition, message):
    if not condition:
        raise EvidenceError(message)


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "Duplicate JSON key: " + key)
        result[key] = value
    return result


def parse_json(data):
    try:
        value = json.loads(data, object_pairs_hook=object_pairs,
            parse_constant=lambda value: (_ for _ in ()).throw(EvidenceError("Non-finite JSON value")))
    except (ValueError, UnicodeError) as error:
        raise EvidenceError("Invalid JSON: " + str(error)) from error
    require(isinstance(value, dict), "Receipt must contain a JSON object")
    return value


def read_json(path):
    return parse_json(path.read_bytes())


def text(value):
    return isinstance(value, str) and bool(value.strip())


def finite(value):
    try:
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    except OverflowError:
        return False


def time_value(value):
    require(isinstance(value, str), "Missing evidence timestamp")
    try:
        stamp = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EvidenceError("Invalid evidence timestamp") from error
    require(stamp.tzinfo is not None, "Evidence timestamps need a timezone")
    return stamp.astimezone(timezone.utc)


def receipt(root, reference):
    require(isinstance(reference, dict), "Missing receipt reference")
    name, expected = reference.get("path"), reference.get("sha256")
    require(isinstance(name, str) and name and not Path(name).is_absolute()
            and ".." not in Path(name).parts, "Receipt paths must stay inside evidence root")
    require(isinstance(expected, str) and SHA.fullmatch(expected), "Invalid receipt digest")
    path = root / name
    require(not any(part.is_symlink() for part in [path, *path.parents] if part != root.parent),
            "Receipt aliases are not accepted")
    require(path.is_file() and path.stat().st_nlink == 1 and path.resolve().is_relative_to(root), "Receipt must be a regular unaliased evidence file")
    require(file_digest(path) == expected, "Receipt bytes differ from recorded digest")
    return path


def document(root, reference):
    data = receipt(root, reference).read_bytes()
    require(hashlib.sha256(data).hexdigest() == reference["sha256"], "Receipt changed while reading")
    return parse_json(data)


def identity(preflight):
    return {"source_sha": preflight["source"]["head"],
            "bundle_tree_sha256": preflight["artifact"]["tree_sha256"],
            "main_executable_sha256": preflight["main_executable_sha256"],
            "bundle_identifier": preflight["bundle_identifier"],
            "bundle_version": preflight["bundle_version"],
            "architectures": preflight["declared_architectures"]}


def check_metrics(root, run, budgets):
    metrics = run.get("metrics")
    require(isinstance(metrics, dict) and set(metrics) == set(budgets["metrics"]), "Performance metrics are incomplete")
    require(run.get("method") == "native-packaged", "Performance must measure the packaged native app")
    require(isinstance(run.get("conditions"), dict) and all(text(run["conditions"].get(key))
        for key in ("machine", "os", "display", "window", "power", "thermal")), "Missing performance conditions")
    scenario = run.get("scenario")
    require(isinstance(scenario, dict) and isinstance(scenario.get("workloads"), list), "Missing matched workload scenarios")
    workloads = scenario["workloads"]
    require(all(isinstance(row, dict) and type(row.get("transcript_messages")) is int and
        type(row.get("sessions")) is int for row in workloads), "Invalid workload matrix")
    require(len(workloads) == 9 and {(row["transcript_messages"], row["sessions"]) for row in workloads}
        == {(messages, sessions) for messages in (1000, 10000, 50000) for sessions in (1, 3, 8)}, "Incomplete workload matrix")
    require(type(scenario.get("navigation_cycles")) is int and scenario["navigation_cycles"] >= 20
        and scenario.get("cold") == "process-cold" and scenario.get("warm") == "process-warm", "Incomplete repeat/startup scenario")
    baseline_pin = run.get("baseline")
    require(isinstance(baseline_pin, dict) and all(isinstance(baseline_pin.get(key), str) and SHA.fullmatch(baseline_pin[key])
        for key in ("bundle_tree_sha256", "main_executable_sha256")), "Missing baseline artifact identity")
    require(baseline_pin["bundle_tree_sha256"] != run["candidate"]["bundle_tree_sha256"] and
        baseline_pin.get("bundle_identifier") == run["candidate"]["bundle_identifier"] and
        baseline_pin.get("architectures") == run["candidate"]["architectures"], "Baseline is not a matched independent artifact")
    candidate_traces = set()
    for name, budget in budgets["metrics"].items():
        require(isinstance(budget, dict), "Invalid performance budget")
        maximum, minimum = budget.get("maximum"), budget.get("minimum_samples")
        require(finite(maximum) and maximum >= 0, "Invalid performance maximum")
        require(isinstance(minimum, int) and not isinstance(minimum, bool) and minimum >= 2, "Performance needs repeated samples")
        metric = metrics[name]
        require(isinstance(metric, dict) and metric.get("unit") == budget.get("unit")
                and isinstance(budget.get("unit"), str) and budget["unit"], "Performance unit differs from budget")
        if name in METRIC_UNITS:
            require(budget["unit"] == METRIC_UNITS[name], "Incorrect metric unit: " + name)
        values = metric.get("samples")
        require(isinstance(values, list) and len(values) >= minimum and all(
            finite(v) and (v >= 0 or name == "navigation-growth-mib") for v in values), "Missing finite performance samples")
        labels = metric.get("sample_workloads")
        require(isinstance(labels, list) and len(labels) == len(values) and all(row in workloads for row in labels)
            and all(labels.count(row) >= minimum for row in workloads), "Missing repeated samples for each workload")
        statistic = budget.get("statistic")
        ordered = sorted(values)
        if statistic == "p95":
            observed = ordered[math.ceil(len(ordered) * .95) - 1]
        elif statistic == "maximum":
            observed = ordered[-1]
        else:
            raise EvidenceError("Unsupported performance statistic")
        require(observed <= maximum, "Performance budget failed: " + name)
        baseline = metric.get("baseline_samples")
        require(isinstance(baseline, list) and len(baseline) >= minimum and all(
            finite(v) and (v >= 0 or name == "navigation-growth-mib") for v in baseline), "Matched baseline samples are missing")
        require(metric.get("baseline_sample_workloads") == labels and len(baseline) == len(labels), "Baseline sample workloads differ")
        traces = []
        for key, candidate, samples in (("raw_trace", run["candidate"], values),
                ("baseline_raw_trace", baseline_pin, baseline)):
            trace = document(root, metric.get(key))
            require(trace.get("schema") == "c3-native-metric-trace/v1" and trace.get("candidate") == candidate
                and trace.get("metric") == name and trace.get("unit") == metric["unit"] and trace.get("samples") == samples
                and trace.get("sample_workloads") == labels and trace.get("conditions") == run["conditions"] and trace.get("scenario") == scenario,
                "Native trace identity, conditions or samples differ")
            require(trace.get("method") in {"xctrace", "native-monotonic-input", "process-rss", "vmmap", "footprint"}, "Unsupported native trace method")
            if name in {"native-hitch-ms-per-s", "paused-cpu-percent"}:
                require(trace["method"] == "xctrace", "Frames and CPU require their native instrument trace")
            receipt(root, trace.get("raw"))
            traces.append(trace)
        require(traces[0]["raw"]["sha256"] != traces[1]["raw"]["sha256"], "Candidate and baseline raw trace bytes must differ")
        candidate_traces.add(traces[0]["raw"]["sha256"])
    return candidate_traces


def validate(root, ledger, repo, app, team_id):
    root = root.resolve(strict=True)
    require(root.is_dir() and not root.is_relative_to(repo.resolve()) and not root.is_relative_to(app.resolve()),
        "Evidence root must be outside source and candidate")
    require(isinstance(ledger, dict), "Ledger must be an object")
    require(ledger.get("schema") == "c3-candidate-evidence/v1", "Unsupported evidence schema")
    preflight = document(root, ledger.get("preflight"))
    require(preflight.get("schema") == "c3-macos-preflight/v1" and preflight.get("stage") == "candidate",
            "Need a Developer ID candidate preflight")
    require(preflight.get("packaging_checks_passed") is True and isinstance(preflight.get("checks"), dict)
            and set(preflight["checks"]) >= CANDIDATE_CHECKS
            and all(value is True for value in preflight["checks"].values()), "Candidate packaging checks have not passed")
    require(isinstance(team_id, str) and re.fullmatch(r"[A-Z0-9]{10}", team_id), "Invalid intended signing team")
    commands = preflight.get("commands")
    require(isinstance(commands, dict) and isinstance(commands.get("signature_metadata"), dict), "Missing signature metadata")
    signature = commands["signature_metadata"]
    require(isinstance(signature.get("stderr"), str) and isinstance(signature.get("stdout"), str), "Malformed signature metadata")
    metadata = signature.get("stderr", "") + signature.get("stdout", "")
    require(signature.get("exit_code") == 0 and ("TeamIdentifier=" + team_id) in metadata.splitlines(), "Candidate belongs to another signing team")
    source, _ = source_snapshot(repo)
    require(not source["status"] and source == preflight["source"], "Source changed after candidate preflight")
    require(artifact_snapshot(app) == preflight["artifact"], "Signed candidate changed after preflight")
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    require(isinstance(info, dict) and text(info.get("CFBundleExecutable")) and
        Path(info["CFBundleExecutable"]).name == info["CFBundleExecutable"], "Invalid bundle executable")
    require(preflight.get("main_executable_sha256") == file_digest(app / "Contents/MacOS" / info["CFBundleExecutable"])
        and preflight.get("bundle_identifier") == info.get("CFBundleIdentifier")
        and preflight.get("bundle_version") == info.get("CFBundleShortVersionString"), "Candidate identity disagrees with actual bundle")
    architectures = preflight.get("declared_architectures")
    require(isinstance(architectures, list) and architectures and all(arch in ("arm64", "x86_64") for arch in architectures)
        and len(set(architectures)) == len(architectures), "Invalid candidate architectures")
    pin = identity(preflight)
    provenance = document(root, ledger.get("build"))
    require(provenance.get("schema") == "c3-build-provenance/v1" and provenance.get("candidate") == pin,
            "Build provenance does not identify this candidate")
    require(provenance.get("lockfile_sha256") == file_digest(repo / "Cargo.lock"), "Build dependency lock differs")
    require(isinstance(provenance.get("command"), list) and provenance["command"]
            and all(text(arg) for arg in provenance["command"])
            and isinstance(provenance.get("toolchain"), dict) and all(text(provenance["toolchain"].get(key))
                for key in ("rustc", "cargo", "tauri", "xcode")), "Missing build command or toolchain")
    receipt(root, provenance.get("log"))
    gates = ledger.get("source_gates")
    require(isinstance(gates, dict) and set(gates) == {"policy-workspace", "process-completion", "durable-events", "native-enforcement"}, "Source gate coverage is incomplete")
    runtime_capabilities = {}
    for name, reference in gates.items():
        gate = document(root, reference)
        require(gate.get("gate") == name and gate.get("status") == "passed"
                and gate.get("source_sha") == pin["source_sha"], "Unqualified source gate: " + name)
        require(isinstance(gate.get("receipts"), list) and gate["receipts"], "Source gate has no verification receipts")
        for ref in gate["receipts"]:
            receipt(root, ref)
        if name == "native-enforcement":
            capabilities = gate.get("runtime_capabilities")
            require(isinstance(capabilities, dict) and set(capabilities) == {"grok", "codex", "claude"}, "Missing native enforcement capability coverage")
            for backend, ref in capabilities.items():
                capability = document(root, ref)
                require(capability.get("schema") == "c3-runtime-capability/v1" and capability.get("source_sha") == pin["source_sha"]
                    and capability.get("backend") == backend and capability.get("status") == "verified",
                    "Native capability receipt identity differs")
                require(isinstance(capability.get("capabilities"), dict) and set(capability["capabilities"]) >= NATIVE_CAPABILITIES
                    and all(value is True for value in capability["capabilities"].values()), "Native policy capabilities remain unavailable")
                pins = capability.get("artifact_pins")
                require(isinstance(pins, dict) and set(pins) >= {"executable", "adapter", "sdk"}
                    and all(isinstance(value, str) and SHA.fullmatch(value) for value in pins.values()), "Missing native artifact pins")
                require(isinstance(capability.get("receipts"), list) and capability["receipts"], "Missing native capability probes")
                for probe in capability["receipts"]:
                    receipt(root, probe)
                runtime_capabilities[backend] = ref
    definitions = ledger.get("definitions")
    require(isinstance(definitions, dict) and set(definitions) == {f"C3-{i:03}" for i in range(1, 43)}, "Need the current first 42 story definitions")
    contract_text = (repo / "docs/release/USER_STORIES.md").read_text()
    contract_revision = re.search(r"^Revision ([1-9][0-9]*)(?:,|\b)", contract_text, re.MULTILINE)
    require(contract_revision and set(definitions) <= set(re.findall(r"C3-[0-9]{3}", contract_text)), "Current source story contract is incomplete")
    for story, definition in definitions.items():
        require(isinstance(definition, dict) and isinstance(definition.get("revision"), int)
                and not isinstance(definition["revision"], bool) and definition["revision"] >= 1, "Invalid story revision: " + story)
        require(definition["revision"] == int(contract_revision.group(1)), "Story revision differs from source contract: " + story)
        receipt(root, definition.get("contract"))
        require(definition["contract"]["sha256"] == file_digest(repo / "docs/release/USER_STORIES.md"), "Story contract differs from current source")
    revision_record = receipt(root, ledger.get("story_revisions"))
    require(file_digest(revision_record) == file_digest(repo / "docs/release/STORY_REVISIONS.md"), "Story revision record differs from current source")
    budgets = document(root, ledger.get("performance_budgets"))
    require(budgets.get("schema") == "c3-performance-budgets/v1" and isinstance(budgets.get("metrics"), dict)
            and set(budgets["metrics"]) >= REQUIRED_METRICS, "Performance budgets are missing required metrics")
    declared = time_value(budgets.get("declared_utc"))
    runs = ledger.get("runs")
    require(isinstance(runs, list) and runs, "No packaged runs recorded")
    seen = set()
    observations_seen = set()
    performance_traces_seen = {}
    coverage = {(story, level): [] for story in definitions for level in REQUIRED_LEVELS[story]}
    cases = {name: [] for names in CASES.values() for name in names}
    for reference in runs:
        run = document(root, reference)
        require(run.get("schema") == "c3-packaged-run/v1" and run.get("candidate") == pin, "Run belongs to another candidate")
        run_id = run.get("run_id")
        require(isinstance(run_id, str) and run_id and run_id not in seen, "Duplicate or missing run identity")
        seen.add(run_id)
        started, finished = time_value(run.get("started_utc")), time_value(run.get("finished_utc"))
        require(started <= finished and finished <= datetime.now(timezone.utc), "Invalid run interval")
        category, case = run.get("category"), run.get("case")
        require(isinstance(category, str) and category in CASES and isinstance(case, str) and case in CASES[category], "Unknown packaged scenario")
        require(isinstance(run.get("status"), str) and run["status"] in {"passed", "failed", "blocked"}, "Invalid run outcome")
        require(isinstance(run.get("receipts"), list) and run["receipts"], "Run lacks observation receipts")
        for ref in run["receipts"]:
            receipt(root, ref)
        stories = run.get("stories")
        require(isinstance(stories, dict) and stories, "Run lacks story coverage")
        levels = run.get("levels")
        require(isinstance(levels, list) and levels and all(isinstance(level, str) for level in levels) and len(set(levels)) == len(levels)
                and set(levels) <= {"packaged", "provider", "recovery", "external"}, "Invalid evidence levels")
        require("packaged" in levels, "Every candidate case requires packaged evidence")
        require(category != "recovery" or "recovery" in levels, "Recovery category requires recovery evidence")
        require(isinstance(run.get("profile"), str) and Path(run["profile"]).is_absolute() and Path(run["profile"]).is_dir()
                and not Path(run["profile"]).is_symlink()
                and Path(run["profile"]).resolve().is_relative_to(root), "Run needs an isolated evidence profile")
        require(isinstance(run.get("observed"), str) and run["observed"].strip(), "Missing observed outcome")
        observation = run.get("observation_receipt")
        receipt(root, observation)
        require(observation["sha256"] not in observations_seen, "Repeated observation bytes cannot establish an independent rerun")
        observations_seen.add(observation["sha256"])
        receipt(root, run.get("fixture"))
        if "provider" in levels:
            provider = run.get("provider")
            require(isinstance(provider, dict) and provider.get("backend") in {"grok", "codex", "claude"}
                    and isinstance(provider.get("model"), str) and provider["model"], "Missing actual provider identity")
            require(provider.get("method") == "actual-native-provider", "Synthetic provider metadata cannot establish provider coverage")
            receipt(root, provider.get("input"))
            receipt(root, provider.get("runtime_capability"))
            require(provider["runtime_capability"]["sha256"] == runtime_capabilities[provider["backend"]]["sha256"],
                "Provider runtime capability differs from verified native enforcement")
        if "external" in levels:
            environment = run.get("external_environment")
            require(isinstance(environment, dict) and environment.get("kind") in {"test-service", "clean-account", "clean-machine"}
                and text(environment.get("identity")), "Missing external service or clean-account evidence")
            receipt(root, environment.get("receipt"))
        if "recovery" in levels:
            recovery = run.get("recovery")
            require(isinstance(recovery, dict) and recovery.get("method") == "owned-process-interruption"
                and text(recovery.get("operation_id")) and text(recovery.get("interruption")), "Missing actual recovery operation evidence")
            receipt(root, recovery.get("before"))
            receipt(root, recovery.get("after"))
        for story, revision in stories.items():
            require(story in definitions and isinstance(revision, int) and not isinstance(revision, bool), "Unknown run story")
            require(1 <= revision <= definitions[story]["revision"], "Run claims an invalid/future story revision")
            if revision == definitions[story]["revision"]:
                for level in REQUIRED_LEVELS[story] & set(levels):
                    coverage[(story, level)].append((started, finished, run["status"], run_id))
                expected_backend = {"C3-009": "grok", "C3-010": "codex", "C3-011": "claude"}.get(story)
                if expected_backend and "provider" in levels:
                    require(run["provider"]["backend"] == expected_backend, "Provider does not match native conversation story")
                if story == "C3-037" and "external" in levels:
                    require(run["external_environment"]["kind"] == "test-service", "Haven requires a test service")
                if story == "C3-039" and "external" in levels:
                    require(run["external_environment"]["kind"] in {"clean-account", "clean-machine"}, "Portable prerequisites require a clean account or machine")
        cases[case].append((started, finished, run["status"], run_id))
        if category == "performance":
            require(declared < started, "Budgets were not declared before measurement")
            if run["status"] == "passed":
                for trace_digest in check_metrics(root, run, budgets):
                    require(trace_digest not in performance_traces_seen, "Repeated native trace cannot establish an independent performance rerun")
                    performance_traces_seen[trace_digest] = run_id
    # Retain adverse outcomes; only subsequent successful repeats resolve them.
    for name, observations in list(coverage.items()) + list(cases.items()):
        failure_end = max((finished for _, finished, outcome, _ in observations if outcome != "passed"), default=None)
        after_failure = []
        # A pass started before a failure completed cannot be a repair rerun.
        # Nor can concurrent copies establish sequential independent repeats.
        previous_end = failure_end
        for started, finished, outcome, run_id in sorted(observations):
            if outcome == "passed" and (previous_end is None or started > previous_end):
                after_failure.append(run_id)
                previous_end = finished
        require(len(after_failure) >= 2, "Needs two passing runs after the latest failure: " + str(name))
    require(source_snapshot(repo)[0] == source and artifact_snapshot(app) == preflight["artifact"],
        "Source or candidate changed during evidence validation")
    # A ledger cannot make an ad-hoc or differently signed bundle acceptable by
    # supplying invented preflight booleans. Reinspect the actual candidate with
    # native tools; this only reads and never signs, installs or submits.
    fresh, _ = inspect_candidate(repo, app, "candidate", architectures)
    require(fresh["packaging_checks_passed"] is True and all(value is True for value in fresh["checks"].values()),
        "Fresh candidate packaging checks failed")
    require(fresh["source"] == source and fresh["artifact"] == preflight["artifact"] and identity(fresh) == pin,
        "Fresh source or signed candidate differs from evidence")
    actual_signature = fresh["commands"]["signature_metadata"]
    actual_metadata = actual_signature["stderr"] + actual_signature["stdout"]
    require(actual_signature["exit_code"] == 0 and ("TeamIdentifier=" + team_id) in actual_metadata.splitlines(),
        "Actual candidate belongs to another signing team")
    return {"schema": "c3-candidate-evidence-result/v1", "candidate": pin,
            "evidence_consistency": "passed", "notarization_evidence_complete": True,
            "observations_independently_verified": False, "human_release_approval": "not_asserted",
            "distribution_upgrade_download": "requires_post_notarization_evidence",
            "run_count": len(runs), "story_count": len(definitions)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-root", required=True, type=Path)
    parser.add_argument("--ledger", required=True, type=Path)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--app", required=True, type=Path)
    parser.add_argument("--team-id", required=True)
    args = parser.parse_args()
    try:
        ledger = read_json(args.ledger)
        require(isinstance(ledger, dict), "Ledger must be an object")
        result = validate(args.evidence_root, ledger, args.repo.resolve(strict=True), args.app.absolute(), args.team_id)
        print(json.dumps(result, indent=2))
        return 0
    except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
        print("Candidate evidence rejected: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
