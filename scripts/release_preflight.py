#!/usr/bin/env python3
"""Read-only macOS artifact checks; writes a new receipt, never signs or submits.

A successful receipt is a packaging check, not product qualification. Capture
again after signing/stapling: those operations change the exact artifact bytes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import subprocess
import sys
from datetime import datetime, timezone


class PreflightError(ValueError):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def run(command, cwd=None):
    try:
        result = subprocess.run(command, cwd=cwd, capture_output=True, timeout=120)
        return {"command": command, "exit_code": result.returncode,
                "stdout": result.stdout.decode("utf-8", errors="replace"),
                "stderr": result.stderr.decode("utf-8", errors="replace")}
    except (OSError, subprocess.TimeoutExpired):
        return {"command": command, "exit_code": None,
                "stdout": "", "stderr": "tool unavailable or timed out"}


def git_bytes(repo, *args):
    try:
        return subprocess.run(["git", *args], cwd=repo, capture_output=True,
                              timeout=120, check=True).stdout
    except (OSError, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
        raise PreflightError("Git source inspection failed") from error


def git(repo, *args):
    return git_bytes(repo, *args).decode("utf-8", errors="surrogateescape")


def source_snapshot(repo):
    # HEAD diff includes staged and unstaged changes, unlike plain git diff.
    patch = git_bytes(repo, "diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD", "--")
    untracked = git(repo, "ls-files", "--others", "--exclude-standard", "-z")
    files = []
    for name in sorted(filter(None, untracked.split("\0"))):
        path = repo / name
        if path.is_symlink() or not path.is_file():
            raise PreflightError("Untracked source must be regular files")
        files.append({"path": name, "sha256": file_digest(path),
                      "mode": stat.S_IMODE(path.stat().st_mode)})
    record = {"head": git(repo, "rev-parse", "HEAD").strip(),
              "branch": git(repo, "branch", "--show-current").strip(),
              "status": git(repo, "status", "--porcelain=v1", "-uall"),
              "patch_sha256": digest(patch), "untracked": files}
    record["snapshot_sha256"] = digest(json.dumps(record, sort_keys=True).encode())
    return record, patch


def artifact_snapshot(app):
    if app.is_symlink() or not app.is_dir() or app.suffix != ".app":
        raise PreflightError("Artifact must be a real .app directory")
    root = app.resolve()
    entries = []
    for path in [app, *sorted(app.rglob("*"))]:
        mode = path.lstat().st_mode
        entry = {"path": path.relative_to(app).as_posix(),
                 "mode": stat.S_IMODE(mode), "xattrs": {}}
        if hasattr(os, "listxattr"):
            for name in sorted(os.listxattr(path, follow_symlinks=False)):
                entry["xattrs"][name] = digest(os.getxattr(path, name, follow_symlinks=False))
        if stat.S_ISLNK(mode):
            target = path.resolve(strict=True)
            if not target.is_relative_to(root):
                raise PreflightError("Artifact contains a symlink outside its bundle")
            entry.update(type="symlink", target=os.readlink(path))
        elif stat.S_ISDIR(mode):
            entry.update(type="directory")
        elif stat.S_ISREG(mode):
            entry.update(type="file", bytes=path.stat().st_size, sha256=file_digest(path))
        else:
            raise PreflightError("Artifact contains a non-file entry")
        entries.append(entry)
    return {"tree_sha256": digest(json.dumps(entries, sort_keys=True).encode()),
            "entries": entries}


def new_output(path, repo, app):
    # Reject dangling links, canonical aliases and output inside inspected inputs.
    if os.path.lexists(path):
        raise PreflightError("Output already exists; choose a new receipt directory")
    resolved = path.resolve()
    if any(resolved.is_relative_to(root.resolve()) for root in (repo, app)):
        raise PreflightError("Receipt directory must be outside the source and app")
    if any(resolved.is_relative_to(root) for root in
           (Path("/Applications").resolve(), Path("/System/Volumes/Data/Applications").resolve(),
            (Path.home() / "Applications").resolve())):
        raise PreflightError("Receipt directory cannot be an application install path")
    return resolved


def developer_id_checks(metadata, entitlements):
    """codesign text is checked only after successful signature verification."""
    return {
        "developer_id": "Authority=Developer ID Application:" in metadata
                        and "Signature=adhoc" not in metadata,
        "team_identifier": bool(re.search(r"^TeamIdentifier=(?!not set$)[A-Z0-9]{10}$",
                                          metadata, re.MULTILINE)),
        "secure_timestamp": bool(re.search(r"^Timestamp=(?!none$|not set$).+$", metadata, re.MULTILINE)),
        "hardened_runtime": bool(re.search(r"flags=0x[0-9a-f]+\([^\n]*\bruntime\b", metadata)),
        "no_debug_entitlement": not entitlements.get("com.apple.security.get-task-allow", False),
    }


def read_entitlements(result):
    try:
        values = plistlib.loads(result["stdout"].encode()) if result["stdout"].strip() else {}
        if result["exit_code"] == 0 and isinstance(values, dict):
            return values, True
    except (plistlib.InvalidFileException, ValueError):
        pass
    return {}, False


def inspect(repo, app, stage, architectures):
    source, patch = source_snapshot(repo)
    tree = artifact_snapshot(app)
    config = json.loads((repo / "src-tauri/tauri.conf.json").read_text())
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    if (not isinstance(config, dict) or not all(isinstance(config.get(key), str) and config[key]
                                               for key in ("identifier", "version"))
            or not isinstance(info, dict)):
        raise PreflightError("Invalid Tauri configuration or bundle metadata")
    executable = info.get("CFBundleExecutable", "")
    if not executable or Path(executable).name != executable or executable in (".", ".."):
        raise PreflightError("Invalid bundle executable name")
    binary = app / "Contents/MacOS" / executable
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise PreflightError("Bundle executable is missing or not executable")
    checks = {"bundle_identifier": info.get("CFBundleIdentifier") == config["identifier"],
              "bundle_version": info.get("CFBundleShortVersionString") == config["version"]}
    commands = {}
    def tool(name, args):
        commands[name] = run(args)
        return commands[name]
    verify = tool("signature", ["codesign", "--verify", "--deep", "--strict", str(app)])
    meta = tool("signature_metadata", ["codesign", "-d", "--verbose=4", str(app)])
    ent = tool("entitlements", ["codesign", "-d", "--entitlements", ":-", str(app)])
    arch = tool("architecture", ["lipo", "-archs", str(binary)])
    sdk = tool("sdk", ["xcrun", "vtool", "-show-build", str(binary)])
    checks["signature_valid"] = verify["exit_code"] == 0
    checks["declared_architectures"] = (arch["exit_code"] == 0 and
        set(arch["stdout"].split()) == set(architectures))
    if stage != "qa":
        checks["clean_source"] = not source["status"]
        checks["signature_metadata_available"] = meta["exit_code"] == 0
        metadata = meta["stderr"] + meta["stdout"]
        checks["signature_identifier"] = ("Identifier=" + config["identifier"]) in metadata.splitlines()
        entitlements, checks["entitlements_readable"] = read_entitlements(ent)
        checks.update(developer_id_checks(metadata, entitlements))
        versions = re.findall(r"^\s*sdk\s+(\d+)\.(\d+)", sdk["stdout"], re.MULTILINE)
        checks["supported_sdk"] = (sdk["exit_code"] == 0 and
                                   len(versions) == len(architectures) and
                                   len(re.findall(r"^\s*platform MACOS$", sdk["stdout"], re.MULTILINE)) == len(architectures) and
                                   all(tuple(map(int, v)) >= (10, 9) for v in versions))
        # Deep verification alone permits an ad-hoc nested signature. Inspect
        # every bundled Mach-O image as well; libraries need no runtime flag.
        magic = {b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
                 b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
                 b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca"}
        for entry in tree["entries"]:
            path = app / entry["path"]
            if entry["type"] != "file" or path == binary:
                continue
            with path.open("rb") as stream:
                if stream.read(4) not in magic:
                    continue
            label = "nested:" + entry["path"]
            nested = tool(label, ["codesign", "-d", "--verbose=4", str(path)])
            nested_ent, readable = read_entitlements(tool(label + ":entitlements",
                ["codesign", "-d", "--entitlements", ":-", str(path)]))
            values = developer_id_checks(nested["stderr"] + nested["stdout"], nested_ent)
            checks[label] = (nested["exit_code"] == 0 and
                             readable and all(values[key] for key in
                                 ("developer_id", "team_identifier", "secure_timestamp", "no_debug_entitlement")))
    if stage == "distribution":
        checks["stapled_ticket"] = tool("ticket", ["xcrun", "stapler", "validate", str(app)])["exit_code"] == 0
        checks["gatekeeper"] = tool("gatekeeper", ["spctl", "--assess", "--type", "execute", "--verbose=4", str(app)])["exit_code"] == 0
    # Do not associate mixed inputs with a seemingly valid result.
    if source_snapshot(repo)[0] != source or artifact_snapshot(app) != tree:
        raise PreflightError("Source or artifact changed during inspection; retry once stable")
    return {"schema": "c3-macos-preflight/v1", "stage": stage,
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "source": source, "app": str(app), "artifact": tree,
            "main_executable_sha256": file_digest(binary),
            "bundle_identifier": info.get("CFBundleIdentifier"),
            "bundle_version": info.get("CFBundleShortVersionString"),
            "declared_architectures": architectures, "checks": checks,
            "packaging_checks_passed": all(checks.values()),
            "product_qualification": "not_asserted",
            "artifact_source_provenance": "not_asserted", "commands": commands}, patch


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--app", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--stage", choices=("qa", "candidate", "distribution"), default="qa")
    parser.add_argument("--architecture", choices=("arm64", "x86_64"), action="append", required=True)
    args = parser.parse_args()
    try:
        repo = args.repo.resolve(strict=True)
        if Path(git(repo, "rev-parse", "--show-toplevel").strip()).resolve() != repo:
            raise PreflightError("--repo must be the Git root")
        output = new_output(args.output, repo, args.app)
        receipt, patch = inspect(repo, args.app.absolute(), args.stage, sorted(set(args.architecture)))
        output.parent.mkdir(parents=True, exist_ok=True)
        output.mkdir(mode=0o700)  # Exclusive create; a raced existing path is never overwritten.
        (output / "source.patch").write_bytes(patch)
        (output / "manifest.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"Receipt: {output / 'manifest.json'}")
        failed = [name for name, passed in receipt["checks"].items() if not passed]
        print("Packaging checks: " + ("failed: " + ", ".join(failed) if failed else "passed"))
        print("Product qualification: not asserted; no signing, submission or installation performed")
        return 1 if failed else 0
    except (PreflightError, OSError, ValueError) as error:
        print(f"Preflight rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
