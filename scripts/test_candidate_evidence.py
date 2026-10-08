"""Failure-focused evidence gate tests, with generated source/bundle/receipts."""
import copy
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import candidate_evidence as evidence
import release_preflight as release


class CandidateEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir()
        (self.repo / "src-tauri").mkdir()
        (self.repo / "docs/release").mkdir(parents=True)
        (self.repo / "src-tauri/tauri.conf.json").write_text('{"identifier":"app.test","version":"1.0"}')
        (self.repo / "Cargo.lock").write_text("generated pinned dependencies")
        (self.repo / "docs/release/USER_STORIES.md").write_text("Revision 1, generated story contract\n" +
            "\n".join(evidence.REQUIRED_LEVELS))
        (self.repo / "docs/release/STORY_REVISIONS.md").write_text("generated revision record")
        def git(*args):
            subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True)
        git("init", "-q")
        git("add", ".")
        git("-c", "user.name=QA", "-c", "user.email=qa@example.invalid", "commit", "-qm", "fixture")
        self.app = self.root / "Example.app"
        (self.app / "Contents/MacOS").mkdir(parents=True)
        (self.app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "app.test", "CFBundleShortVersionString": "1.0", "CFBundleExecutable": "example"}))
        self.binary = self.app / "Contents/MacOS/example"
        self.binary.write_bytes(b"generated non-native binary")
        self.binary.chmod(0o755)
        self.store = self.root / "evidence"
        self.store.mkdir()
        self.profile = self.store / "profile"
        self.profile.mkdir()
        self.now = datetime.now(timezone.utc) - timedelta(days=1)
        with patch.object(release, "run", side_effect=self.tools):
            preflight = release.inspect(self.repo, self.app, "candidate", ["arm64"])[0]
        self.pin = evidence.identity(preflight)
        raw = self.write("raw.txt", b"generated raw trace, not native evidence")
        self.ledger = {"schema": "c3-candidate-evidence/v1", "preflight": self.write("preflight.json", preflight),
            "build": self.write("build.json", {"schema": "c3-build-provenance/v1", "candidate": self.pin,
                "lockfile_sha256": release.file_digest(self.repo / "Cargo.lock"), "command": ["fixture-build"],
                "toolchain": {key: "fixture-version" for key in ("rustc", "cargo", "tauri", "xcode")},
                "log": raw}), "source_gates": {}, "definitions": {}, "runs": []}
        for gate in ("policy-workspace", "process-completion", "durable-events", "native-enforcement"):
            gate_record = {"gate": gate, "status": "passed", "source_sha": self.pin["source_sha"], "receipts": [raw]}
            if gate == "native-enforcement":
                gate_record["runtime_capabilities"] = {backend: self.write(f"capability-{backend}.json", {
                    "schema": "c3-runtime-capability/v1", "backend": backend, "source_sha": self.pin["source_sha"],
                    "status": "verified", "capabilities": {name: True for name in evidence.NATIVE_CAPABILITIES},
                    "artifact_pins": {name: "c" * 64 for name in ("executable", "adapter", "sdk")}, "receipts": [raw]})
                    for backend in ("grok", "codex", "claude")}
            self.ledger["source_gates"][gate] = self.write(gate + ".json", gate_record)
        contract = self.write("stories.md", (self.repo / "docs/release/USER_STORIES.md").read_bytes())
        self.ledger["story_revisions"] = self.write("revisions.md", (self.repo / "docs/release/STORY_REVISIONS.md").read_bytes())
        for story in evidence.REQUIRED_LEVELS:
            self.ledger["definitions"][story] = {"revision": 1, "contract": contract}
        budgets = {"schema": "c3-performance-budgets/v1", "declared_utc": self.now.isoformat(),
            "metrics": {name: {"maximum": 10, "minimum_samples": 2, "unit": evidence.METRIC_UNITS[name], "statistic": "p95"} for name in evidence.REQUIRED_METRICS}}
        self.ledger["performance_budgets"] = self.write("budgets.json", budgets)
        self.raw = raw
        for category, cases in evidence.CASES.items():
            for case in sorted(cases):
                for _ in range(2):
                    self.add_run(category, case, {"C3-001": 1}, ["packaged", "recovery"] if category == "recovery" else ["packaged"])
        for story, levels in evidence.REQUIRED_LEVELS.items():
            for _ in range(2):
                self.add_run("workflow", "operation-policy", {story: 1}, sorted(levels))

    def tools(self, command, cwd=None):
        output, stderr = "", ""
        if command[0] == "lipo":
            output = "arm64\n"
        elif command[0] == "codesign" and "--verbose=4" in command:
            stderr = "Authority=Developer ID Application: Fixture\nIdentifier=app.test\nTeamIdentifier=ABCDEFGHIJ\nTimestamp=fixture-time\nCodeDirectory v=20500 flags=0x10000(runtime)\n"
        elif "--entitlements" in command:
            output = plistlib.dumps({}).decode()
        elif "vtool" in command:
            output = " platform MACOS\n sdk 26.2\n"
        return {"command": command, "exit_code": 0, "stdout": output, "stderr": stderr}

    def write(self, name, value):
        path = self.store / name
        path.write_bytes(value if isinstance(value, bytes) else (json.dumps(value, indent=2) + "\n").encode())
        return {"path": name, "sha256": release.file_digest(path)}

    def add_run(self, category, case, stories, levels, status="passed"):
        count = len(self.ledger["runs"])
        started = self.now + timedelta(minutes=count + 1)
        run = {"schema": "c3-packaged-run/v1", "candidate": self.pin, "run_id": f"run-{count}",
            "started_utc": started.isoformat(), "finished_utc": (started + timedelta(seconds=20)).isoformat(),
            "category": category, "case": case, "status": status, "receipts": [self.raw], "stories": stories,
            "levels": levels, "profile": str(self.profile), "observed": "generated fixture observation",
            "observation_receipt": self.write(f"observation-{count}.txt", f"independent generated observation {count}".encode()), "fixture": self.raw}
        if "provider" in levels:
            story = next(iter(stories))
            backend = {"C3-009": "grok", "C3-010": "codex", "C3-011": "claude"}.get(story, "grok")
            run["provider"] = {"backend": backend, "model": "fixture", "method": "actual-native-provider",
                "input": self.raw, "runtime_capability": json.loads((self.store / "native-enforcement.json").read_text())["runtime_capabilities"][backend]}
        if "external" in levels:
            run["external_environment"] = {"kind": "test-service" if "C3-037" in stories else "clean-account",
                "identity": "generated external fixture", "receipt": self.raw}
        if "recovery" in levels:
            run["recovery"] = {"method": "owned-process-interruption", "operation_id": f"generated-operation-{count}",
                "interruption": "generated kill fixture", "before": self.raw, "after": self.raw}
        if category == "performance":
            run.update(method="native-packaged", conditions={key: "generated-condition" for key in
                ("machine", "os", "display", "window", "power", "thermal")},
                scenario={"workloads": [{"transcript_messages": messages, "sessions": sessions}
                    for messages in (1000, 10000, 50000) for sessions in (1, 3, 8)],
                    "navigation_cycles": 20, "cold": "process-cold", "warm": "process-warm"},
                baseline={"bundle_tree_sha256": "a" * 64, "main_executable_sha256": "b" * 64,
                    "bundle_identifier": self.pin["bundle_identifier"], "architectures": self.pin["architectures"]}, metrics={})
            for name in evidence.REQUIRED_METRICS:
                labels = [row for row in run["scenario"]["workloads"] for _ in range(2)]
                metric = {"unit": evidence.METRIC_UNITS[name], "samples": [1, 2] * 9, "baseline_samples": [2, 3] * 9,
                    "sample_workloads": labels, "baseline_sample_workloads": labels}
                for key, candidate, samples in (("raw_trace", self.pin, metric["samples"]),
                        ("baseline_raw_trace", run["baseline"], metric["baseline_samples"])):
                    trace = {"schema": "c3-native-metric-trace/v1", "candidate": candidate, "metric": name,
                        "unit": metric["unit"], "samples": samples, "sample_workloads": labels,
                        "conditions": run["conditions"], "scenario": run["scenario"],
                        "method": "xctrace", "raw": self.write(f"raw-{count}-{name}-{key}.txt",
                            f"generated distinct trace fixture {count} {name} {key}".encode())}
                    metric[key] = self.write(f"trace-{count}-{name}-{key}.json", trace)
                run["metrics"][name] = metric
        reference = self.write(f"run-{count}.json", run)
        self.ledger["runs"].append(reference)
        return reference

    def edit_run(self, index, change):
        reference = self.ledger["runs"][index]
        run = json.loads((self.store / reference["path"]).read_text())
        change(run)
        self.ledger["runs"][index] = self.write(reference["path"], run)

    def validate(self, ledger=None, team="ABCDEFGHIJ", tools=None):
        # Generated bundles are deliberately not signed apps. Mock native
        # inspection explicitly; passing fixtures assert consistency only.
        with patch.object(release, "run", side_effect=tools or self.tools):
            return evidence.validate(self.store, ledger or self.ledger, self.repo, self.app, team)

    def test_complete_fixture_checks_consistency_without_claiming_observations_or_release_approval(self):
        result = self.validate()
        self.assertTrue(result["notarization_evidence_complete"])
        self.assertFalse(result["observations_independently_verified"])
        self.assertEqual(result["human_release_approval"], "not_asserted")

    def test_missing_source_gate_and_wrong_team_rejected(self):
        missing = copy.deepcopy(self.ledger)
        del missing["source_gates"]["native-enforcement"]
        with self.assertRaisesRegex(evidence.EvidenceError, "coverage"):
            self.validate(missing)
        with self.assertRaisesRegex(evidence.EvidenceError, "another signing team"):
            self.validate(team="ZYXWVUTSRQ")

    def test_changed_source_or_signed_candidate_invalidates_results(self):
        self.binary.write_bytes(b"changed signed payload")
        with self.assertRaisesRegex(evidence.EvidenceError, "candidate changed"):
            self.validate()
        self.binary.write_bytes(b"generated non-native binary")
        (self.repo / "Cargo.lock").write_text("changed dependency input")
        with self.assertRaisesRegex(evidence.EvidenceError, "Source changed"):
            self.validate()

    def test_receipt_tampering_and_escape_are_rejected(self):
        (self.store / "raw.txt").write_bytes(b"tampered trace")
        with self.assertRaisesRegex(evidence.EvidenceError, "Receipt bytes"):
            self.validate()
        with self.assertRaisesRegex(evidence.EvidenceError, "inside evidence"):
            evidence.receipt(self.store, {"path": "../outside", "sha256": "a" * 64})

    def test_missing_repeat_and_latest_failure_need_two_new_passes(self):
        self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"], "failed")
        with self.assertRaisesRegex(evidence.EvidenceError, "two passing"):
            self.validate()
        self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"])
        with self.assertRaisesRegex(evidence.EvidenceError, "two passing"):
            self.validate()
        self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"])
        self.assertTrue(self.validate()["notarization_evidence_complete"])

    def test_generated_packaged_evidence_cannot_replace_provider_level(self):
        for index in range(len(self.ledger["runs"])):
            self.edit_run(index, lambda run: run.update(levels=[level for level in run["levels"] if level != "provider"]))
        with self.assertRaisesRegex(evidence.EvidenceError, "provider"):
            self.validate()

    def test_generated_packaged_evidence_cannot_replace_external_level(self):
        for index in range(len(self.ledger["runs"])):
            self.edit_run(index, lambda run: run.update(levels=[level for level in run["levels"] if level != "external"]))
        with self.assertRaisesRegex(evidence.EvidenceError, "external"):
            self.validate()

    def test_duplicate_run_or_observation_does_not_count_as_repeat(self):
        self.edit_run(1, lambda run: run.update(run_id="run-0"))
        with self.assertRaisesRegex(evidence.EvidenceError, "Duplicate"):
            self.validate()
        first = json.loads((self.store / "run-0.json").read_text())
        self.edit_run(1, lambda run: run.update(run_id="run-1", observation_receipt=first["observation_receipt"]))
        with self.assertRaisesRegex(evidence.EvidenceError, "independent rerun"):
            self.validate()

    def test_stale_revision_requires_new_runs(self):
        self.ledger["definitions"]["C3-017"]["revision"] = 2
        with self.assertRaisesRegex(evidence.EvidenceError, "C3-017"):
            self.validate()

    def test_performance_requires_prior_budgets_finite_samples_baseline_and_raw_trace(self):
        index = next(i for i, reference in enumerate(self.ledger["runs"]) if "metrics" in json.loads((self.store / reference["path"]).read_text()))
        original = json.loads((self.store / self.ledger["runs"][index]["path"]).read_text())
        for mutation in (lambda run: run["metrics"]["input-ms"].update(samples=[1, 100]),
                         lambda run: run["metrics"]["input-ms"].update(baseline_samples=[]),
                         lambda run: run["metrics"]["input-ms"].pop("raw_trace"),
                         lambda run: run.update(method="fake-animation-frames")):
            self.ledger["runs"][index] = self.write(f"run-{index}.json", copy.deepcopy(original))
            self.edit_run(index, mutation)
            with self.subTest(mutation=mutation), self.assertRaises(evidence.EvidenceError):
                self.validate()

    def test_duplicate_json_keys_and_malformed_nested_metadata_are_rejected(self):
        duplicate = self.write("duplicate.json", b'{"status":"failed","status":"passed"}')
        with self.assertRaisesRegex(evidence.EvidenceError, "Duplicate JSON"):
            evidence.document(self.store, duplicate)
        preflight = json.loads((self.store / "preflight.json").read_text())
        for mutation in (lambda value: value.update(checks=[True]),
                         lambda value: value.update(checks={"one": True}),
                         lambda value: value["commands"].update(signature_metadata={"stderr": 1, "stdout": ""})):
            changed = copy.deepcopy(preflight)
            mutation(changed)
            self.ledger["preflight"] = self.write("preflight.json", changed)
            with self.subTest(mutation=mutation), self.assertRaises(evidence.EvidenceError):
                self.validate()

    def test_claimed_executable_identity_must_match_actual_bundle(self):
        preflight = json.loads((self.store / "preflight.json").read_text())
        preflight["main_executable_sha256"] = "a" * 64
        self.ledger["preflight"] = self.write("preflight.json", preflight)
        with self.assertRaisesRegex(evidence.EvidenceError, "actual bundle"):
            self.validate()

    def test_passes_started_before_failure_completed_do_not_resolve_failure(self):
        failed = self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"], "failed")
        failure = json.loads((self.store / failed["path"]).read_text())
        end = evidence.time_value(failure["started_utc"]) + timedelta(hours=1)
        self.edit_run(len(self.ledger["runs"]) - 1, lambda run: run.update(finished_utc=end.isoformat()))
        self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"])
        self.add_run("workflow", "reviewed-change", {"C3-017": 1}, ["packaged"])
        with self.assertRaisesRegex(evidence.EvidenceError, "two passing"):
            self.validate()

    def test_recovery_and_provider_levels_require_specific_observation_metadata(self):
        for level, field in (("recovery", "recovery"), ("provider", "provider")):
            index = next(i for i, reference in enumerate(self.ledger["runs"])
                if level in json.loads((self.store / reference["path"]).read_text())["levels"])
            original = json.loads((self.store / self.ledger["runs"][index]["path"]).read_text())
            self.edit_run(index, lambda run: run.pop(field))
            with self.subTest(level=level), self.assertRaises(evidence.EvidenceError):
                self.validate()
            self.ledger["runs"][index] = self.write(f"run-{index}.json", original)

    def test_baseline_must_match_conditions_scenario_and_distinct_raw_trace(self):
        index = next(i for i, reference in enumerate(self.ledger["runs"]) if "metrics" in json.loads((self.store / reference["path"]).read_text()))
        original = json.loads((self.store / self.ledger["runs"][index]["path"]).read_text())
        reference = original["metrics"]["input-ms"]["baseline_raw_trace"]
        trace = json.loads((self.store / reference["path"]).read_text())
        for mutation in (lambda value: value["conditions"].update(display="different-display"),
                         lambda value: value.update(raw=json.loads((self.store / original["metrics"]["input-ms"]["raw_trace"]["path"]).read_text())["raw"]),
                         lambda value: value.update(candidate=self.pin)):
            changed = copy.deepcopy(trace)
            mutation(changed)
            self.edit_run(index, lambda run: run["metrics"]["input-ms"].update(
                baseline_raw_trace=self.write(reference["path"], changed)))
            with self.subTest(mutation=mutation), self.assertRaises(evidence.EvidenceError):
                self.validate()
            self.ledger["runs"][index] = self.write(f"run-{index}.json", copy.deepcopy(original))

    def test_native_gate_cannot_pass_with_unavailable_runtime_capability(self):
        capability = json.loads((self.store / "capability-codex.json").read_text())
        capability["capabilities"]["immutable-read-only"] = False
        gate = json.loads((self.store / "native-enforcement.json").read_text())
        gate["runtime_capabilities"]["codex"] = self.write("capability-codex.json", capability)
        self.ledger["source_gates"]["native-enforcement"] = self.write("native-enforcement.json", gate)
        with self.assertRaisesRegex(evidence.EvidenceError, "remain unavailable"):
            self.validate()

    def test_reused_native_trace_cannot_establish_performance_repeat(self):
        indices = [i for i, reference in enumerate(self.ledger["runs"]) if "metrics" in json.loads((self.store / reference["path"]).read_text())]
        first = json.loads((self.store / self.ledger["runs"][indices[0]]["path"]).read_text())
        self.edit_run(indices[1], lambda run: run.update(metrics=first["metrics"]))
        with self.assertRaisesRegex(evidence.EvidenceError, "independent performance"):
            self.validate()

    def test_budget_timestamp_and_native_workload_coverage_are_required(self):
        budget = json.loads((self.store / "budgets.json").read_text())
        budget["declared_utc"] = datetime.now(timezone.utc).isoformat()
        self.ledger["performance_budgets"] = self.write("budgets.json", budget)
        with self.assertRaisesRegex(evidence.EvidenceError, "before measurement"):
            self.validate()
        budget["declared_utc"] = self.now.isoformat()
        self.ledger["performance_budgets"] = self.write("budgets.json", budget)
        index = next(i for i, reference in enumerate(self.ledger["runs"]) if "metrics" in json.loads((self.store / reference["path"]).read_text()))
        self.edit_run(index, lambda run: run["metrics"]["input-ms"].update(sample_workloads=[]))
        with self.assertRaisesRegex(evidence.EvidenceError, "each workload"):
            self.validate()

    def test_frame_metric_cannot_use_rss_measurement_method(self):
        index = next(i for i, reference in enumerate(self.ledger["runs"]) if "metrics" in json.loads((self.store / reference["path"]).read_text()))
        run = json.loads((self.store / self.ledger["runs"][index]["path"]).read_text())
        reference = run["metrics"]["native-hitch-ms-per-s"]["raw_trace"]
        trace = json.loads((self.store / reference["path"]).read_text())
        trace["method"] = "process-rss"
        self.edit_run(index, lambda run: run["metrics"]["native-hitch-ms-per-s"].update(raw_trace=self.write(reference["path"], trace)))
        with self.assertRaisesRegex(evidence.EvidenceError, "native instrument"):
            self.validate()

    def test_cases_need_packaged_and_recovery_categories_need_recovery_level(self):
        first = json.loads((self.store / "run-0.json").read_text())
        self.edit_run(0, lambda run: run.update(levels=["external"], external_environment={
            "kind": "clean-account", "identity": "generated", "receipt": self.raw}))
        with self.assertRaisesRegex(evidence.EvidenceError, "requires packaged"):
            self.validate()
        self.ledger["runs"][0] = self.write("run-0.json", first)
        index = next(i for i, reference in enumerate(self.ledger["runs"])
            if json.loads((self.store / reference["path"]).read_text())["category"] == "recovery")
        self.edit_run(index, lambda run: run.update(levels=["packaged"]))
        with self.assertRaisesRegex(evidence.EvidenceError, "Recovery category"):
            self.validate()

    def test_provider_capability_must_be_the_verified_backend_receipt(self):
        index = next(i for i, reference in enumerate(self.ledger["runs"])
            if "provider" in json.loads((self.store / reference["path"]).read_text())["levels"])
        self.edit_run(index, lambda run: run["provider"].update(runtime_capability=self.raw))
        with self.assertRaisesRegex(evidence.EvidenceError, "differs from verified"):
            self.validate()

    def test_forged_preflight_cannot_qualify_actual_adhoc_bundle_or_other_team(self):
        def actual_adhoc(command, cwd=None):
            result = self.tools(command, cwd)
            if command[0] == "codesign" and "--verbose=4" in command:
                result["stderr"] = "Identifier=app.test\nSignature=adhoc\nTeamIdentifier=not set\n"
            return result
        with self.assertRaisesRegex(evidence.EvidenceError, "Fresh candidate packaging"):
            self.validate(tools=actual_adhoc)
        def other_team(command, cwd=None):
            result = self.tools(command, cwd)
            result["stderr"] = result["stderr"].replace("ABCDEFGHIJ", "ZYXWVUTSRQ")
            return result
        with self.assertRaisesRegex(evidence.EvidenceError, "Actual candidate belongs"):
            self.validate(tools=other_team)


if __name__ == "__main__":
    unittest.main()
