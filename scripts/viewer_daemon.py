#!/usr/bin/env python3
"""
Lightweight server-side daemon for Infant Hand Motion Viewer.
Communicates via standard input/output with the remote desktop viewer.

Protocol (requests are served concurrently and their responses interleave, matched up by "id"):
- Requests arrive as JSON lines on stdin: {"id": <int>, "cmd": "<command>", ...}
- Text responses are written as JSON lines on stdout: {"id": <int>, "status": "ok"|"error", ...}\n
- Binary responses (get_file, get_video) write a JSON header line:
    {"id": <int>, "status": "ok", "size": <byte_length>, ...}\n
  followed by the payload in chunk frames, each a JSON line {"id": <int>, "chunk": <n>}\n and then n raw bytes,
  until <byte_length> bytes have been sent. At most WINDOW_CHUNKS chunks of a response are in flight: the client
  sends {"cmd": "ack", "id": <int>} for every chunk it has taken, which frees the daemon to send another. That keeps
  a large download from queueing up in front of the small replies of other requests.
"""

import hashlib
import json
import math
import os
import re
import struct
import sys
import threading
import traceback
from pathlib import Path


def natural_sort_key(s: str):
    """Sort strings with embedded numbers naturally (e.g. T1 before T10)."""
    return [int(text) if text.isdigit() else text.lower() for text in re.split(r"(\d+)", s)]


HEADER_SIZE = 20
HEXPORT_VERSION = 4


def read_hexport_header(path: str):
    """The JSON metadata header of a version 4 .hexport file as a dict; None if unreadable or not version 4."""
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


def _score(value):
    """A quality score as a float, or None when it is null, missing or NaN."""
    if not isinstance(value, (int, float)) or isinstance(value, bool) or math.isnan(value):
        return None
    return float(value)


def read_hexport_metadata(path: str):
    """Subject, trial and the left and right trial quality scores (None if null, absent or NaN) from a .hexport
    file's JSON header."""
    meta = read_hexport_header(path)
    if not meta:
        return None, None, None, None
    return (
        meta.get("subject") or None,
        meta.get("trial") or None,
        _score(meta.get("quality_trial_score_left")),
        _score(meta.get("quality_trial_score_right")),
    )


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
                subject, trial, score_left, score_right = read_hexport_metadata(entry.path)
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
                        "quality_score_left": score_left,
                        "quality_score_right": score_right,
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


CHUNK_SIZE = 128 * 1024
WINDOW_CHUNKS = 8
ACK_WAIT_SECONDS = 120


class Replier:
    """Writes responses to stdout one whole frame at a time, so replies from concurrent requests interleave."""

    def __init__(self, out):
        self.out = out
        self.lock = threading.Lock()
        self.windows = {}
        self.windows_lock = threading.Lock()

    def json(self, obj):
        line = (json.dumps(obj) + "\n").encode("utf-8")
        with self.lock:
            self.out.write(line)
            self.out.flush()

    def payload(self, req_id, header, chunks):
        """Send `header` (which carries the total "size") and then the chunks, at most WINDOW_CHUNKS unacknowledged."""
        window = threading.Semaphore(WINDOW_CHUNKS)
        with self.windows_lock:
            self.windows[req_id] = window
        try:
            self.json(header)
            try:
                for chunk in chunks:
                    if not window.acquire(timeout=ACK_WAIT_SECONDS):
                        return  # the client stopped acknowledging; it is gone
                    frame = (json.dumps({"id": req_id, "chunk": len(chunk)}) + "\n").encode("utf-8")
                    with self.lock:
                        self.out.write(frame)
                        self.out.write(chunk)
                        self.out.flush()
            except OSError as e:
                # The header promised more bytes than will come, so the client has to be told
                self.json({"id": req_id, "status": "error", "message": f"Failed while sending: {e}"})
        finally:
            with self.windows_lock:
                self.windows.pop(req_id, None)

    def ack(self, req_id):
        with self.windows_lock:
            window = self.windows.get(req_id)
        if window:
            window.release()


def chunks_of_bytes(data):
    for start in range(0, len(data), CHUNK_SIZE):
        yield data[start:start + CHUNK_SIZE]


def chunks_of_file(path, size):
    with open(path, "rb") as f:
        remaining = size
        while remaining > 0:
            chunk = f.read(min(CHUNK_SIZE, remaining))
            if not chunk:
                break
            remaining -= len(chunk)
            yield chunk


def handle_get_file(req, reply):
    path_str = os.path.expanduser(req.get("path", ""))
    if not os.path.isfile(path_str):
        reply.json({"id": req["id"], "status": "error", "message": f"File not found: {path_str}"})
        return

    try:
        st = os.stat(path_str)
        digest = hash_cache().hash_of(path_str, st.st_size, st.st_mtime_ns)
        hash_cache().save()
        if req.get("if_none_match") == digest:
            reply.json({"id": req["id"], "status": "not_modified", "hash": digest})
            return
        with open(path_str, "rb") as f:
            data = f.read()
        reply.payload(
            req["id"], {"id": req["id"], "status": "ok", "size": len(data), "hash": digest}, chunks_of_bytes(data)
        )
    except Exception as e:
        reply.json({"id": req["id"], "status": "error", "message": str(e)})


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
        self.lock = threading.Lock()
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
        with self.lock:
            self.entries[file_path] = [size, mtime_ns, digest]
            self.dirty = True
        return digest

    def save(self):
        with self.lock:
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
_hash_cache_lock = threading.Lock()
_found_videos = {}


def hash_cache():
    global _hash_cache
    with _hash_cache_lock:
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


def handle_get_video(req, reply):
    export_path_str = os.path.expanduser(req.get("path") or req.get("export_path", ""))
    meta = read_hexport_header(export_path_str)
    video_hash = (meta or {}).get("video_hash") or ""
    basename = (meta or {}).get("video_basename")
    if not meta or not video_hash:
        reply.json({"id": req["id"], "status": "error", "message": f"Not a version 4 hand export: {export_path_str}"})
        return
    size = meta.get("video_size_bytes")
    roots = [r for r in req.get("roots") or [] if isinstance(r, str) and r.strip()]
    video = find_video_by_hash(video_hash, roots, basename, size if isinstance(size, int) else None)
    if not video:
        reply.json({
            "id": req["id"], "status": "not_found", "video_hash": video_hash, "video_basename": basename,
        })
        return
    video = Path(video)
    kind = req.get("kind")
    if kind in DEBUG_VIDEO_PREFIX:
        video = debug_video_path(kind, video)
        if not video.exists():
            reply.json({"id": req["id"], "status": "not_found", "video_hash": video_hash, "video_basename": basename})
            return
        video_hash = DEBUG_VIDEO_PREFIX[kind] + video_hash[2:]
    try:
        resolved = video.resolve()
        size = resolved.stat().st_size
        reply.payload(
            req["id"],
            {"id": req["id"], "status": "ok", "size": size, "filename": video.name, "video_hash": video_hash},
            chunks_of_file(resolved, size),
        )
    except Exception as e:
        reply.json({"id": req["id"], "status": "error", "message": str(e)})


def serve(req, reply):
    """Answer one request; runs on its own thread so a slow request does not hold up the others."""
    req_id = req.get("id", 0)
    cmd = req.get("cmd", "")
    try:
        if cmd == "scan_tree":
            res = handle_scan_tree(req)
            res["id"] = req_id
            reply.json(res)
        elif cmd == "get_file":
            handle_get_file(req, reply)
        elif cmd == "get_video":
            handle_get_video(req, reply)
        else:
            reply.json({"id": req_id, "status": "error", "message": f"Unknown command: {cmd}"})
    except Exception as e:
        traceback.print_exc(file=sys.stderr)
        reply.json({"id": req_id, "status": "error", "message": str(e)})


def main():
    stdin = sys.stdin
    reply = Replier(sys.stdout.buffer)

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
            reply.json({"status": "error", "message": f"Invalid JSON: {e}"})
            continue

        req_id = req.get("id", 0)
        cmd = req.get("cmd", "")
        if cmd == "ack":
            reply.ack(req_id)
        elif cmd == "ping":
            reply.json({"id": req_id, "status": "ok", "version": 2})
        elif cmd == "quit" or cmd == "exit":
            reply.json({"id": req_id, "status": "ok"})
            break
        else:
            threading.Thread(target=serve, args=(req, reply), daemon=True).start()


if __name__ == "__main__":
    main()
