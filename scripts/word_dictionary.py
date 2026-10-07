#!/usr/bin/env python3
"""Bounded, provider-free reader for Bomb Code's pinned dictionary snapshot.

Coordinates and alignments are source records, not independent meaning labels.
The command line accepts only JSON on stdin, never filenames or provider options.
"""
import collections
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import unicodedata

SCHEMA = "bomb-code/dictionary-shapes/v1"
REFERENCE_ROOT = Path("/Users/paulcooper/.grok/control-panel/wizard-joe/reference")
MANIFEST_SHA = "4d466d7d8e830f6a3330e619a497f99aa3b6fa6c7439432c610b1f3485498e83"
MANIFEST_PATH = "round_trip_experiment/PACKAGE_MANIFEST.json"
GRAPH_PATH = "semantic_e8/outputs/aligned_graph.json"
MODEL_PATH = "semantic_e8/outputs/model.json"
SENSESNAP_PATHS = ("semantic_e8/sense_snap.py", "sensesnap/src/sensesnap/place.py", "sensesnap/src/sensesnap/store.py")
LANGUAGES = ("eng", "jpn", "ind", "zsm", "cmn")
MAX_INPUT = 16 * 1024
MAX_OUTPUT = 1024 * 1024
MAX_MANIFEST = 1024 * 1024
MAX_GRAPH = 192 * 1024 * 1024
MAX_MODEL = 48 * 1024 * 1024


def normalize(text):
    return " ".join(unicodedata.normalize("NFC", text).replace("_", " ").casefold().split())


def strict_json(raw):
    def reject_constant(value):
        raise ValueError("Non-finite JSON numbers are unsupported")
    return json.loads(raw, parse_constant=reject_constant)


def canonical_hash(value):
    digest = hashlib.sha256()
    encoder = json.JSONEncoder(ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
    for piece in encoder.iterencode(value):
        digest.update(piece.encode("utf-8"))
    return digest.hexdigest()


def read_member(root, relative, maximum, expected_hash=None, expected_bytes=None):
    path = root / relative
    # Resolve containment and require a regular file; root identity is host-controlled.
    if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError("Pinned dictionary member unavailable")
    with path.open("rb") as handle:
        before = os.fstat(handle.fileno())
        if before.st_size > maximum or (expected_bytes is not None and before.st_size != expected_bytes):
            raise ValueError("Pinned dictionary member size invalid")
        raw = handle.read(maximum + 1)
        after = os.fstat(handle.fileno())
    if len(raw) > maximum or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError("Pinned dictionary member changed during read")
    digest = hashlib.sha256(raw).hexdigest()
    if expected_hash is not None and digest != expected_hash:
        raise ValueError("Pinned dictionary member hash changed")
    return raw, digest


def read_pinned(root, relative, maximum, expected_hash=None, expected_bytes=None):
    raw, digest = read_member(root, relative, maximum, expected_hash, expected_bytes)
    # Escape only non-ASCII UTF-8 runs without materializing a full wide Unicode
    # graph string. CPython otherwise widens this 149 MB file to four bytes per
    # character when even one astral character occurs. JSON escaping preserves
    # the decoded strings, including non-BMP letters and emoji.
    escaped = bytearray()
    cursor = 0
    for match in re.finditer(rb"[\x80-\xff]+", raw):
        escaped.extend(raw[cursor:match.start()])
        escaped.extend(json.dumps(match.group().decode("utf-8"), ensure_ascii=True)[1:-1].encode("ascii"))
        cursor = match.end()
    escaped.extend(raw[cursor:])
    del raw
    text = escaped.decode("ascii")
    del escaped
    return strict_json(text), digest


def bounded_integer(value, name, maximum):
    if type(value) is not int or value < 0 or value > maximum:
        raise ValueError(name + " outside supported bounds")
    return value


def request_options(request):
    if not isinstance(request, dict):
        raise ValueError("Request must be an object")
    allowed = {"action", "referenceRoot", "query", "language", "offset", "limit", "counterpartOffset", "counterpartLimit"}
    if set(request) - allowed:
        raise ValueError("Unsupported request fields")
    if request.get("referenceRoot") != str(REFERENCE_ROOT):
        raise ValueError("Only the installed frozen dictionary root is supported")
    action = request.get("action", "query")
    if action not in ("query", "coverage"):
        raise ValueError("Unsupported dictionary action")
    query = request.get("query", "")
    if not isinstance(query, str) or len(query) > 256 or "\x00" in query:
        raise ValueError("Query must contain at most 256 characters")
    language = request.get("language", "all")
    if language not in ("all",) + LANGUAGES:
        raise ValueError("Unsupported encoded language")
    offset = bounded_integer(request.get("offset", 0), "offset", 100000)
    limit = bounded_integer(request.get("limit", 20), "limit", 20)
    counterpart_offset = bounded_integer(request.get("counterpartOffset", 0), "counterpartOffset", 100000)
    counterpart_limit = bounded_integer(request.get("counterpartLimit", 10), "counterpartLimit", 20)
    if not limit or not counterpart_limit:
        raise ValueError("Page limits must be positive")
    return action, query, language, offset, limit, counterpart_offset, counterpart_limit


class Dictionary:
    def __init__(self, graph, model, reference, verified_graph_hash=None):
        if graph.get("schema") != "semantic-e8-alignment-graph/v1" or model.get("schema") != "semantic-e8/typed-relation-fit/v1":
            raise ValueError("Unsupported pinned dictionary schema")
        if model.get("source_graph_hash") != (verified_graph_hash or canonical_hash(graph)):
            raise ValueError("Dictionary graph/model identity mismatch")
        self.reference = reference
        self.reference.update({"graphHash": model["source_graph_hash"], "modelId": model["model_id"], "modelSnapshotHash": canonical_hash(model)})
        self.senses = self.unique(graph["senses"], "id", 50000)
        self.concepts = self.unique(graph["concepts"], "id", 10000)
        self.placements = self.unique(model["placements"], "concept_id", 10000)
        self.unplaced = self.unique(model.get("unplaced_concepts", []), "concept_id", 10000)
        self.sources = self.unique(graph["sources"], "id", 100)
        self.alignments = collections.defaultdict(list)
        self.by_concept = collections.defaultdict(set)
        alignments = self.unique(graph["alignments"], "id", 100000)
        for row in alignments.values():
            sense_id = row.get("sense_id")
            if sense_id not in self.senses:
                raise ValueError("Alignment references unknown sense")
            concept_id = row.get("concept_id")
            if concept_id is not None and concept_id not in self.concepts:
                raise ValueError("Alignment references unknown concept")
            self.alignments[sense_id].append(row)
            if concept_id:
                self.by_concept[concept_id].add(sense_id)
        for sense_id, sense in self.senses.items():
            if sense.get("language") not in LANGUAGES:
                raise ValueError("Unsupported sense language in frozen dictionary")
            for concept_id in sense.get("candidate_concept_ids", []):
                if concept_id not in self.concepts:
                    raise ValueError("Sense references unknown concept")
                self.by_concept[concept_id].add(sense_id)
        for placement in self.placements.values():
            if placement["concept_id"] not in self.concepts:
                raise ValueError("Placement references unknown concept")
            for key in ("position8", "direction8", "root_direction8", "scaled_anchor8", "residual8", "direction_residual8"):
                values = placement.get(key)
                if values is not None and (not isinstance(values, list) or len(values) != 8 or any(type(v) not in (int, float) or not math.isfinite(v) for v in values)):
                    raise ValueError("Invalid native placement vector")
        self.coverage = self.census()

    @staticmethod
    def unique(rows, key, maximum):
        if not isinstance(rows, list) or len(rows) > maximum:
            raise ValueError("Dictionary inventory exceeds supported bounds")
        result = {}
        for row in rows:
            if not isinstance(row, dict) or not isinstance(row.get(key), str) or row[key] in result:
                raise ValueError("Dictionary inventory identity invalid")
            result[row[key]] = row
        return result

    @classmethod
    def load(cls):
        manifest, manifest_hash = read_pinned(REFERENCE_ROOT, MANIFEST_PATH, MAX_MANIFEST, MANIFEST_SHA)
        members = {row["path"]: row for row in manifest["files"]}
        def member(relative, maximum, parse=True):
            record = members.get(relative)
            if not isinstance(record, dict):
                raise ValueError("Dictionary member missing from pinned manifest")
            reader = read_pinned if parse else read_member
            return reader(REFERENCE_ROOT, relative, maximum, record["sha256"], record["bytes"])
        graph, graph_file_hash = member(GRAPH_PATH, MAX_GRAPH)
        graph_hash = canonical_hash(graph)
        # The entire graph identity has been checked. Release unrelated relations
        # before parsing the model; the browser only needs the lexical inventory.
        graph = {key: graph[key] for key in ("schema", "sources", "senses", "concepts", "alignments")}
        model, model_file_hash = member(MODEL_PATH, MAX_MODEL)
        implementations = {relative: member(relative, 1024 * 1024, False)[1] for relative in SENSESNAP_PATHS}
        return cls(graph, model, {
            "manifestSha256": manifest_hash, "graphFileSha256": graph_file_hash,
            "modelFileSha256": model_file_hash, "graphPath": GRAPH_PATH, "modelPath": MODEL_PATH,
            "senseSnapImplementationHashes": implementations,
            "scope": "Encoded frozen inventory; not every word in these languages",
        }, verified_graph_hash=graph_hash)

    def concept_ids(self, sense):
        return sorted(set(sense.get("candidate_concept_ids", [])) | {a["concept_id"] for a in self.alignments[sense["id"]] if a.get("concept_id")})

    def census(self):
        counts = {lang: {"senses": 0, "senseLocalDefinitions": 0, "missingSenseLocalDefinitions": 0, "withFittedConcept": 0, "withoutFittedConcept": 0} for lang in LANGUAGES}
        for sense in self.senses.values():
            entry = counts[sense["language"]]
            entry["senses"] += 1
            has_definition = bool(isinstance(sense.get("definition"), str) and sense["definition"].strip())
            entry["senseLocalDefinitions" if has_definition else "missingSenseLocalDefinitions"] += 1
            fitted = any(cid in self.placements for cid in self.concept_ids(sense))
            entry["withFittedConcept" if fitted else "withoutFittedConcept"] += 1
        return {"totalSenses": len(self.senses), "totalConcepts": len(self.concepts),
                "fittedConcepts": len(self.placements), "unplacedConcepts": len(self.concepts) - len(self.placements),
                "senseLocalDefinitions": sum(c["senseLocalDefinitions"] for c in counts.values()),
                "missingSenseLocalDefinitions": sum(c["missingSenseLocalDefinitions"] for c in counts.values()),
                "languages": counts, "geometrySelectsSense": False, "alignmentIsIndependentGold": False}

    def source(self, source_id):
        row = self.sources.get(source_id, {})
        return {key: row[key] for key in ("id", "url", "version", "language", "license", "sha256", "relative_file") if key in row}

    def counterpart(self, sense_id, shared, origin_id):
        sense = self.senses[sense_id]
        shared_ids = sorted(set(self.concept_ids(sense)) & set(shared))
        def equivalents(sid):
            return {a["concept_id"] for a in self.alignments[sid] if a.get("concept_id") and a.get("kind") == "equivalent" and a.get("asserted") is True}
        equivalent_ids = sorted(set(shared_ids) & equivalents(sense_id) & equivalents(origin_id))
        return {"senseId": sense_id, "language": sense["language"], "lemma": sense["lemma"],
                "sourceId": sense.get("source_id"), "sourceRecordId": sense.get("source_record_id"),
                "sharedConceptIds": shared_ids,
                "linkStatus": "source-equivalent" if equivalent_ids else "shared-concept-candidate",
                "assertedEquivalentSharedConceptIds": equivalent_ids,
                "candidateSharedConceptIds": sorted(set(shared_ids) - set(equivalent_ids)),
                "alignmentIds": [a["id"] for a in self.alignments[sense_id] if a.get("concept_id") in shared],
                "evidenceRefs": sense.get("evidence_refs", []),
                "alignments": [a for a in self.alignments[sense_id] if a.get("concept_id") in shared],
                "scope": "Shared concept links; inspect alignment kind and asserted status; not independent bilingual gold"}

    def hit(self, sense, counterpart_offset, counterpart_limit):
        sense_id = sense["id"]
        definition = sense.get("definition")
        status = "sense-local" if isinstance(definition, str) and definition.strip() else "missing-sense-local-definition"
        concept_ids = self.concept_ids(sense)
        counterparts = sorted({sid for cid in concept_ids for sid in self.by_concept[cid]
                               if self.senses[sid]["language"] != sense["language"]})
        selected = counterparts[counterpart_offset:counterpart_offset + counterpart_limit]
        concepts = []
        for cid in concept_ids:
            concept = self.concepts[cid]
            placement = self.placements.get(cid)
            native = None if placement is None else {key: placement[key] for key in (
                "concept_id", "status", "position8", "direction8", "root_id", "root_index", "root_ties",
                "root_direction8", "scaled_anchor8", "residual8", "direction_residual8", "root_angular_error", "radius", "hierarchy_level") if key in placement}
            concepts.append({"conceptId": cid, "label": concept.get("label"),
                "sharedConceptGloss": concept.get("definition"), "sharedConceptGlossLanguage": concept.get("definition_language", "eng" if cid.startswith("pwn30:") else None),
                "definitionsByLanguage": concept.get("definitions_by_language", {}),
                "evidenceRefs": concept.get("evidence_refs", []), "sourceIds": concept.get("source_ids", []),
                "placementStatus": "fitted" if placement else "unavailable", "placement": native,
                "unavailableReason": None if placement else self.unplaced.get(cid, {}).get("reason", "No fitted placement in pinned model")})
        return {"senseId": sense_id, "language": sense["language"], "lemma": sense["lemma"], "pos": sense.get("pos"),
                "definition": definition, "definitionLanguage": sense.get("definition_language"), "definitionStatus": status,
                "sourceId": sense.get("source_id"), "sourceRecordId": sense.get("source_record_id"),
                "source": self.source(sense.get("source_id")), "evidenceRefs": sense.get("evidence_refs", []),
                "graphLocator": GRAPH_PATH + "#/senses/id=" + sense_id,
                "conceptIds": concept_ids, "alignments": self.alignments[sense_id], "concepts": concepts,
                "counterpartCount": len(counterparts), "counterpartOffset": counterpart_offset,
                "counterpartLimit": counterpart_limit, "counterparts": [self.counterpart(sid, concept_ids, sense_id) for sid in selected],
                "counterpartsTruncated": len(selected) < len(counterparts),
                "nextCounterpartOffset": counterpart_offset + len(selected) if counterpart_offset + len(selected) < len(counterparts) else None,
                "senseSnapStatus": "unavailable-without-sentence-interpretation", "sentenceUseStatus": "unavailable-without-sentence-interpretation"}

    def respond(self, options):
        action, query, language, offset, limit, counterpart_offset, counterpart_limit = options
        term = normalize(query)
        query_kind = "literal"
        inverse_ids = None
        if query.startswith("sense:"):
            query_kind = "sense-id"
        elif query.startswith("e8-root:"):
            query_kind = "root-id"
            # Roots are exact native addresses. Invalid or noncanonical addresses
            # have no matches; nearby roots are never substituted.
            inverse_ids = {cid for cid, placement in self.placements.items()
                           if placement.get("root_id") == query}
        elif query in self.concepts or query.startswith(("pwn30:", "idwik-concept:")):
            query_kind = "concept-id"
            inverse_ids = {query} if query in self.concepts else set()
        inverse_senses = None if inverse_ids is None else {sid for cid in inverse_ids for sid in self.by_concept[cid]}
        matches = []
        if action == "query":
            for sense in self.senses.values():
                if language != "all" and sense["language"] != language:
                    continue
                if query_kind == "sense-id":
                    matched = sense["id"] == query
                elif inverse_senses is not None:
                    matched = sense["id"] in inverse_senses
                else:
                    fields = (sense["id"], sense["lemma"], sense.get("definition") or "")
                    matched = not term or any(term in normalize(field) for field in fields)
                if matched:
                    matches.append(sense)
            # Exact form matches precede literal definition matches, stable by source identity.
            matches.sort(key=lambda s: (normalize(s["id"]) != term, normalize(s["lemma"]) != term, s["language"], normalize(s["lemma"]), s["id"]))
        selected = matches[offset:offset + limit]
        return {"schema": SCHEMA, "status": "ready", "action": action, "reference": self.reference,
                "coverage": self.coverage, "query": query, "queryKind": query_kind, "language": language,
                "inverseNotice": ("Same native root does not establish synonymy or equivalence; inspect distinct concept and sense identities and their source alignments." if query_kind == "root-id" else
                                  "Concept links retain candidate versus asserted source alignment records; a link does not independently establish meaning equivalence." if query_kind == "concept-id" else None),
                "offset": offset, "limit": limit, "resultCount": len(matches),
                "returnedCount": len(selected), "truncated": len(selected) < len(matches),
                "hasMore": offset + len(selected) < len(matches),
                "nextOffset": offset + len(selected) if offset + len(selected) < len(matches) else None,
                "hits": [self.hit(s, counterpart_offset, counterpart_limit) for s in selected],
                "authority": {"toolsDispatched": False, "approvalsGranted": False, "memoryCommitted": False}}


def encode_response(response):
    raw = json.dumps(response, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    if len(raw) + 1 > MAX_OUTPUT:
        raise ValueError("Dictionary response exceeds 1 MiB; reduce the page limit")
    return raw + b"\n"


def main():
    try:
        raw = sys.stdin.buffer.read(MAX_INPUT + 1)
        if len(raw) > MAX_INPUT:
            raise ValueError("Dictionary request exceeds 16 KiB")
        options = request_options(strict_json(raw))
        response = Dictionary.load().respond(options)
        output = encode_response(response)
    except (ValueError, KeyError, TypeError, OSError, UnicodeError, RecursionError) as error:
        output = encode_response({"schema": SCHEMA, "status": "unavailable", "error": str(error)[:300], "hits": []})
    sys.stdout.buffer.write(output)


if __name__ == "__main__":
    main()
