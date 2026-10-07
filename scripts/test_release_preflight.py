"""Failure-focused release checks using generated source and app fixtures."""
import json
import importlib.util
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_preflight", Path(__file__).with_name("release_preflight.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


METADATA = """Authority=Developer ID Application: Example (ABCDEFGHIJ)
Identifier=app.test
TeamIdentifier=ABCDEFGHIJ
Timestamp=Oct 7, 2026 at 10:00:00
CodeDirectory v=20500 flags=0x10000(runtime)
"""


class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        def git(*args):
            subprocess.run(["git", *args], cwd=self.repo, capture_output=True, check=True)
        git("init", "-q")
        (self.repo / "src-tauri").mkdir()
        (self.repo / "src-tauri/tauri.conf.json").write_text(json.dumps({"identifier": "app.test", "version": "1.0"}))
        (self.repo / "example").write_text("original")
        git("add", ".")
        git("-c", "user.name=QA", "-c", "user.email=qa@example.invalid", "commit", "-qm", "fixture")
        self.app = self.root / "Example.app"
        (self.app / "Contents/MacOS").mkdir(parents=True)
        (self.app / "Contents/Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "app.test", "CFBundleShortVersionString": "1.0", "CFBundleExecutable": "example"}))
        self.binary = self.app / "Contents/MacOS/example"
        self.binary.write_bytes(b"generated fixture")
        self.binary.chmod(0o755)

    def fake_tools(self, command, cwd=None):
        if command[0] == "git":
            return self.real_run(command, cwd)
        output = ""
        error = ""
        if command[0] == "lipo":
            output = "arm64\n"
        elif "--verbose=4" in command and command[0] == "codesign":
            error = METADATA
        elif "--entitlements" in command:
            output = plistlib.dumps({}).decode()
        elif "vtool" in command:
            output = " platform MACOS\n  sdk 26.2\n"
        return {"command": command, "exit_code": 0, "stdout": output, "stderr": error}

    def inspect(self, stage="candidate", tools=None):
        self.real_run = release.run
        with patch.object(release, "run", side_effect=tools or self.fake_tools):
            return release.inspect(self.repo, self.app, stage, ["arm64"])[0]

    def test_clean_candidate_is_only_packaging_check(self):
        record = self.inspect()
        self.assertTrue(record["packaging_checks_passed"])
        self.assertEqual(record["product_qualification"], "not_asserted")
        self.assertEqual(record["artifact_source_provenance"], "not_asserted")

    def test_dirty_source_blocked_but_qa_records_patch(self):
        (self.repo / "example").write_text("changed")
        (self.repo / "new source").write_text("untracked")
        candidate = self.inspect()
        self.assertFalse(candidate["checks"]["clean_source"])
        self.assertEqual(len(candidate["source"]["untracked"]), 1)
        self.assertTrue(self.inspect("qa")["packaging_checks_passed"])
        self.assertNotEqual(candidate["source"]["patch_sha256"], release.digest(b""))

    def test_staged_changes_captured(self):
        (self.repo / "example").write_text("staged")
        subprocess.run(["git", "add", "example"], cwd=self.repo, check=True)
        record, difference = release.source_snapshot(self.repo)
        self.assertIn(b"staged", difference)
        self.assertEqual(record["patch_sha256"], release.digest(difference))

    def test_non_utf8_patch_bytes_preserved(self):
        (self.repo / "example").write_bytes(b"changed: \xff\n")
        record, difference = release.source_snapshot(self.repo)
        expected = subprocess.run(["git", "diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD", "--"],
                                  cwd=self.repo, capture_output=True, check=True).stdout
        self.assertEqual(difference, expected)
        self.assertEqual(record["patch_sha256"], release.digest(expected))

    def test_metadata_failures(self):
        cases = [METADATA.replace("Developer ID Application:", "Apple Development:"),
                 METADATA.replace("Timestamp=Oct 7, 2026 at 10:00:00", "Timestamp=none"),
                 METADATA.replace("0x10000(runtime)", "0x2(adhoc)"),
                 METADATA.replace("TeamIdentifier=ABCDEFGHIJ", "TeamIdentifier=not set")]
        for metadata in cases:
            with self.subTest(metadata=metadata):
                self.assertFalse(all(release.developer_id_checks(metadata, {}).values()))
        self.assertFalse(release.developer_id_checks(METADATA, {"com.apple.security.get-task-allow": True})["no_debug_entitlement"])

    def test_tool_failure_is_not_success(self):
        def tools(command, cwd=None):
            record = self.fake_tools(command, cwd)
            if command[0] == "codesign":
                record["exit_code"] = 1
            return record
        self.assertFalse(self.inspect(tools=tools)["packaging_checks_passed"])

    def test_wrong_architecture_or_sdk_blocks(self):
        def tools(command, cwd=None):
            record = self.fake_tools(command, cwd)
            if command[0] == "lipo":
                record["stdout"] = "x86_64\n"
            if "vtool" in command:
                record["stdout"] = " platform MACOS\n  sdk 10.8\n"
            return record
        checks = self.inspect(tools=tools)["checks"]
        self.assertFalse(checks["declared_architectures"])
        self.assertFalse(checks["supported_sdk"])

    def test_notarized_stage_requires_ticket_and_gatekeeper(self):
        def tools(command, cwd=None):
            record = self.fake_tools(command, cwd)
            if "stapler" in command or command[0] == "spctl":
                record["exit_code"] = 1
            return record
        checks = self.inspect("distribution", tools)["checks"]
        self.assertFalse(checks["stapled_ticket"])
        self.assertFalse(checks["gatekeeper"])

    def test_nested_adhoc_image_blocks(self):
        nested = self.app / "Contents/MacOS/helper"
        nested.write_bytes(b"\xcf\xfa\xed\xfe" + b"generated helper")
        def tools(command, cwd=None):
            record = self.fake_tools(command, cwd)
            if str(nested) in command:
                record["stderr"] = "Signature=adhoc\n"
            return record
        self.assertFalse(self.inspect(tools=tools)["checks"]["nested:Contents/MacOS/helper"])

    def test_bundle_metadata_mismatch_blocks(self):
        (self.repo / "src-tauri/tauri.conf.json").write_text('{"identifier":"other","version":"2.0"}')
        checks = self.inspect()["checks"]
        self.assertFalse(checks["bundle_identifier"])
        self.assertFalse(checks["bundle_version"])

    def test_executable_path_traversal_rejected(self):
        info = self.app / "Contents/Info.plist"
        info.write_bytes(plistlib.dumps({"CFBundleExecutable": "../outside"}))
        with self.assertRaises(release.PreflightError):
            self.inspect()

    def test_external_symlink_rejected(self):
        (self.app / "outside").symlink_to(self.repo / "example")
        with self.assertRaises(release.PreflightError):
            release.artifact_snapshot(self.app)

    def test_bundle_bytes_and_modes_change_hash(self):
        first = release.artifact_snapshot(self.app)["tree_sha256"]
        self.binary.chmod(0o700)
        second = release.artifact_snapshot(self.app)["tree_sha256"]
        self.binary.write_bytes(b"changed")
        third = release.artifact_snapshot(self.app)["tree_sha256"]
        self.assertEqual(len({first, second, third}), 3)

    def test_new_output_refuses_existing_and_aliases(self):
        existing = self.root / "existing"
        existing.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(self.repo, target_is_directory=True)
        dangling = self.root / "dangling"
        dangling.symlink_to(self.root / "missing")
        for path in [existing, dangling, self.repo / "report", self.app / "report", alias / "report", Path("/Applications/new-report")]:
            with self.subTest(path=path), self.assertRaises(release.PreflightError):
                release.new_output(path, self.repo, self.app)
        self.assertEqual(release.new_output(self.root / "receipt", self.repo, self.app), (self.root / "receipt").resolve())

    def test_artifact_change_during_inspection_rejected(self):
        changed = False
        def tools(command, cwd=None):
            nonlocal changed
            record = self.fake_tools(command, cwd)
            if command[0] == "lipo" and not changed:
                self.binary.write_bytes(b"changed during check")
                changed = True
            return record
        with self.assertRaises(release.PreflightError):
            self.inspect(tools=tools)

    def test_source_change_during_inspection_rejected(self):
        def tools(command, cwd=None):
            record = self.fake_tools(command, cwd)
            if command[0] == "lipo":
                (self.repo / "example").write_text("changed during check")
            return record
        with self.assertRaises(release.PreflightError):
            self.inspect(tools=tools)


class InstallerTests(unittest.TestCase):
    script = Path(__file__).with_name("install.sh")

    def test_default_invocation_refuses_before_build(self):
        result = subprocess.run(["bash", str(self.script)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage:", result.stderr)

    def test_existing_destination_never_replaced(self):
        with tempfile.TemporaryDirectory(suffix=".app") as folder:
            path = Path(folder)
            sentinel = path / "preserve"
            sentinel.write_text("original")
            result = subprocess.run(["bash", str(self.script), "--development", "--destination", folder], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(sentinel.read_text(), "original")

    def test_notary_credentials_are_not_echoed_or_used(self):
        env = dict(os.environ, APPLE_PASSWORD="test-secret-do-not-print")
        result = subprocess.run(["bash", str(self.script), "--development", "--destination", "/tmp/unused-test.app"], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("notarization environment", result.stderr)
        self.assertNotIn(env["APPLE_PASSWORD"], result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
