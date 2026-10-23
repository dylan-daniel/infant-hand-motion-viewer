#!/usr/bin/env python3
"""
Lightweight server-side daemon for Infant Hand Motion Viewer.
Communicates via standard input/output with the remote desktop viewer.

Protocol:
- Requests arrive as JSON lines on stdin: {"id": <int>, "cmd": "<command>", ...}
- Text responses are written as JSON lines on stdout: {"id": <int>, "status": "ok"|"error", ...}\n
- Binary responses (get_file, get_video) write a JSON header line:
    {"id": <int>, "status": "ok", "size": <byte_length>, ...}\n
  followed immediately by exactly <byte_length> raw bytes.
"""

import hashlib
import json
import math
import os
import re
import struct
import sys
import traceback
from pathlib import Path


def natural_sort_key(s: str):
    """Sort strings with embedded numbers naturally (e.g. T1 before T10)."""
    return [int(text) if text.isdigit() else text.lower() for text in re.split(r"(\d+)", s)]


HEADER_SIZE = 20
HEXPORT_VERSION = 3


def read_hexport_header(path: str):
    """The JSON metadata header of a version 3 .hexport file as a dict; None if unreadable or not version 3."""
    try:
        with open(path, "rb") as fp:
            header = fp.read(HEADER_SIZE)
            if len(header) < HEADER_SIZE or header[:4] != b"HEXP":
                return None
            version, _payload, _compressed, meta_size = struct.unpack("<IIII", header[4:HEADER_SIZE])
            if version != HEXPORT_VERSION:
                return None
            meta = json.loads(fp.read(meta_size).decode("utf-8"))
        return meta if isinstance(meta, dict) else None
    except Exception:
        return None


def read_hexport_metadata(path: str):
    """Subject, trial and trial quality score (None if absent or NaN) from a .hexport file's JSON header."""
    meta = read_hexport_header(path)
    if not meta:
        return None, None, None
    score = meta.get("quality_trial_score")
    if not isinstance(score, (int, float)) or isinstance(score, bool) or math.isnan(score):
        score = None
    return meta.get("subject") or None, meta.get("trial") or None, score


def _disambiguate_names(files_list):
    seen = {}
    for f in files_list:
        seen[f["name"]] = seen.get(f["name"], 0) + 1
    for f in files_list:
        if seen[f["name"]] > 1 and f.get("params"):
            short_p = f["params"][:6]
            f["name"] = f"{f['name']} ({short_p})"
        f.pop("params", None)


def scan_tree(root_path_str: str):
    """Recursively scan directory, returning a pruned synthetic tree grouped by subject and trial."""
    root_path = os.path.expanduser(root_path_str)
    if not os.path.isdir(root_path):
        return None

    def _scan(dir_path: str):
        subdirs = []
        hexport_entries = []
        try:
            with os.scandir(dir_path) as it:
                for entry in it:
                    if entry.name.startswith("."):
                        continue
                    try:
                        if entry.is_dir(follow_symlinks=False):
                            sub = _scan(entry.path)
                            if sub:
                                subdirs.append(sub)
                        elif entry.is_file(follow_symlinks=False) and entry.name.lower().endswith(".hexport"):
                            hexport_entries.append(entry)
                    except OSError:
                        continue
        except (PermissionError, OSError):
            pass

        children = []

        if hexport_entries:
            subject_groups = {}
            unclassified = []

            for entry in hexport_entries:
                subject, trial, quality_score = read_hexport_metadata(entry.path)
                stem = Path(entry.name).stem
                parts = stem.split("__")
                fps_tag = parts[1] if len(parts) >= 2 else ""
                params_tag = parts[2] if len(parts) >= 3 else ""

                if subject and trial:
                    display_name = f"{trial}_{fps_tag}" if fps_tag else trial
                    subject_groups.setdefault(subject, []).append({
                        "name": display_name,
                        "path": entry.path,
                        "is_file": True,
                        "children": [],
                        "params": params_tag,
                        "quality_score": quality_score,
                    })
                else:
                    unclassified.append({
                        "name": stem,
                        "path": entry.path,
                        "is_file": True,
                        "children": []
                    })

            folder_basename = os.path.basename(dir_path)

            if len(subject_groups) == 1 and list(subject_groups.keys())[0] == folder_basename:
                single_subject_files = list(subject_groups.values())[0]
                _disambiguate_names(single_subject_files)
                children.extend(single_subject_files)
            else:
                for subj in sorted(subject_groups.keys(), key=natural_sort_key):
                    s_files = subject_groups[subj]
                    _disambiguate_names(s_files)
                    s_files.sort(key=lambda c: natural_sort_key(c["name"]))
                    children.append({
                        "name": subj,
                        "path": os.path.join(dir_path, subj),
                        "is_file": False,
                        "children": s_files
                    })

            if unclassified:
                unclassified.sort(key=lambda c: natural_sort_key(c["name"]))
                children.extend(unclassified)

        if subdirs:
            children.extend(subdirs)

        if children:
            children.sort(key=lambda c: (c["is_file"], natural_sort_key(c["name"])))
            folder_name = os.path.basename(dir_path) or dir_path
            return {
                "name": folder_name,
                "path": dir_path,
                "is_file": False,
                "children": children
            }
        return None

    tree = _scan(root_path)
    if tree is None:
        folder_name = os.path.basename(root_path) or root_path
        tree = {
            "name": folder_name,
            "path": root_path,
            "is_file": False,
            "children": []
        }
    return tree


def handle_scan_tree(req):
    root = req.get("root", "")
    tree = scan_tree(root)
    if tree is None:
        return {"status": "error", "message": f"Directory not found or unreadable: {root}"}
    return {"status": "ok", "tree": tree}


def handle_get_file(req, out):
    path_str = os.path.expanduser(req.get("path", ""))
    if not os.path.isfile(path_str):
        write_json_line(out, {"id": req["id"], "status": "error", "message": f"File not found: {path_str}"})
        return

    try:
        with open(path_str, "rb") as f:
            data = f.read()
        write_json_line(out, {"id": req["id"], "status": "ok", "size": len(data)})
        out.write(data)
        out.flush()
    except Exception as e:
        write_json_line(out, {"id": req["id"], "status": "error", "message": str(e)})


VIDEO_EXTENSIONS = (".mp4", ".m4v", ".mov", ".mkv", ".avi", ".webm")
HASH_CACHE_PATH = Path(os.environ.get("VIEWER_HASH_CACHE", Path.home() / ".cache" / "infant_hand_motion_viewer" / "video_hashes.json"))


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


class HashCache:
    """Remembers path -> (size, mtime_ns, sha256) on disk so a video is not rehashed unless it changed."""

    def __init__(self, path):
        self.path = Path(path)
        self.entries = {}
        self.dirty = False
        try:
            with open(self.path, encoding="utf-8") as f:
                self.entries = json.load(f)
        except (OSError, ValueError):
            pass

    def lookup(self, file_path: str, size: int, mtime_ns: int):
        entry = self.entries.get(file_path)
        if entry and entry[0] == size and entry[1] == mtime_ns:
            return entry[2]
        return None

    def hash_of(self, file_path: str, size: int, mtime_ns: int):
        cached = self.lookup(file_path, size, mtime_ns)
        if cached:
            return cached
        digest = sha256_file(file_path)
        self.entries[file_path] = [size, mtime_ns, digest]
        self.dirty = True
        return digest

    def save(self):
        if not self.dirty:
            return
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_suffix(".tmp")
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(self.entries, f)
            os.replace(tmp, self.path)
            self.dirty = False
        except OSError:
            pass


_hash_cache = None
_found_videos = {}


def hash_cache():
    global _hash_cache
    if _hash_cache is None:
        _hash_cache = HashCache(HASH_CACHE_PATH)
    return _hash_cache


def list_video_files(roots):
    """(path, size, mtime_ns) of every video file under `roots`, in root order."""
    files = []
    for root in roots:
        for dirpath, _, names in os.walk(os.path.expanduser(root)):
            for name in names:
                if not name.lower().endswith(VIDEO_EXTENSIONS):
                    continue
                full = os.path.join(dirpath, name)
                try:
                    st = os.stat(full)
                except OSError:
                    continue
                if st.st_size > 0:
                    files.append((full, st.st_size, st.st_mtime_ns))
    return files


def find_video_by_hash(video_hash: str, roots, basename=None, size=None):
    """
    Path of a video under `roots` whose SHA-256 equals `video_hash`, or None.
    Files whose size differs from `size` are skipped without hashing, files named `basename` are hashed first,
    and every hash is remembered by path, size and modification time.
    """
    if not video_hash or not roots:
        return None
    key = (video_hash, tuple(roots))
    cache = hash_cache()
    known = _found_videos.get(key)
    if known:
        try:
            st = os.stat(known)
            if cache.lookup(known, st.st_size, st.st_mtime_ns) == video_hash:
                return known
        except OSError:
            pass
        del _found_videos[key]
    files = list_video_files(roots)
    for full, file_size, mtime in files:
        if cache.lookup(full, file_size, mtime) == video_hash:
            _found_videos[key] = full
            return full
    candidates = [f for f in files if size is None or f[1] == size]
    candidates.sort(key=lambda f: os.path.basename(f[0]) != basename)
    try:
        for full, file_size, mtime in candidates:
            try:
                if cache.hash_of(full, file_size, mtime) == video_hash:
                    _found_videos[key] = full
                    return full
            except OSError:
                continue
    finally:
        cache.save()
    return None


DEBUG_VIDEO_ROOT = Path("/HDD16TB/infant_grasp_workspace_outputs")
DEBUG_VIDEO_PREFIX = {"wilor": "5a", "sam3": "5b"}


def debug_video_path(kind: str, source: Path):
    subject, trial = source.parent.name, "_".join(source.stem.split("_")[-2:])
    if kind == "wilor":
        return DEBUG_VIDEO_ROOT / "wilor_hamer_compare" / subject / trial / f"{subject}_{trial}_wilor.mp4"
    return DEBUG_VIDEO_ROOT / "sam3_render" / subject / f"{trial}.mp4"


def handle_get_video(req, out):
    export_path_str = os.path.expanduser(req.get("path") or req.get("export_path", ""))
    meta = read_hexport_header(export_path_str)
    video_hash = (meta or {}).get("video_hash") or ""
    basename = (meta or {}).get("video_basename")
    if not meta or not video_hash:
        write_json_line(out, {"id": req["id"], "status": "error", "message": f"Not a version 3 hand export: {export_path_str}"})
        return
    size = meta.get("video_size_bytes")
    roots = [r for r in req.get("roots") or [] if isinstance(r, str) and r.strip()]
    video = find_video_by_hash(video_hash, roots, basename, size if isinstance(size, int) else None)
    if not video:
        write_json_line(out, {
            "id": req["id"], "status": "not_found", "video_hash": video_hash, "video_basename": basename,
        })
        return
    video = Path(video)
    kind = req.get("kind")
    if kind in DEBUG_VIDEO_PREFIX:
        video = debug_video_path(kind, video)
        if not video.exists():
            write_json_line(out, {"id": req["id"], "status": "not_found", "video_hash": video_hash, "video_basename": basename})
            return
        video_hash = DEBUG_VIDEO_PREFIX[kind] + video_hash[2:]
    try:
        resolved = video.resolve()
        size = resolved.stat().st_size
        write_json_line(out, {
            "id": req["id"],
            "status": "ok",
            "size": size,
            "filename": video.name,
            "video_hash": video_hash,
        })
        with open(resolved, "rb") as f:
            remaining = size
            while remaining > 0:
                chunk = f.read(min(1 << 20, remaining))
                if not chunk:
                    break
                out.write(chunk)
                remaining -= len(chunk)
        out.flush()
    except Exception as e:
        write_json_line(out, {"id": req["id"], "status": "error", "message": str(e)})


def write_json_line(out, obj):
    line = json.dumps(obj) + "\n"
    out.write(line.encode("utf-8"))
    out.flush()


def main():
    stdin = sys.stdin
    stdout = sys.stdout.buffer

    # Optional handshake banner to verify connection
    sys.stderr.write("infant_hand_motion_viewer daemon started\n")
    sys.stderr.flush()

    for raw_line in stdin:
        line = raw_line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception as e:
            write_json_line(stdout, {"status": "error", "message": f"Invalid JSON: {e}"})
            continue

        req_id = req.get("id", 0)
        cmd = req.get("cmd", "")

        try:
            if cmd == "ping":
                write_json_line(stdout, {"id": req_id, "status": "ok", "version": 1})
            elif cmd == "scan_tree":
                res = handle_scan_tree(req)
                res["id"] = req_id
                write_json_line(stdout, res)
            elif cmd == "get_file":
                handle_get_file(req, stdout)
            elif cmd == "get_video":
                handle_get_video(req, stdout)
            elif cmd == "quit" or cmd == "exit":
                write_json_line(stdout, {"id": req_id, "status": "ok"})
                break
            else:
                write_json_line(stdout, {"id": req_id, "status": "error", "message": f"Unknown command: {cmd}"})
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            write_json_line(stdout, {"id": req_id, "status": "error", "message": str(e)})


if __name__ == "__main__":
    main()
