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
import os
import re
import struct
import sys
import traceback
import zlib
from pathlib import Path


def natural_sort_key(s: str):
    """Sort strings with embedded numbers naturally (e.g. T1 before T10)."""
    return [int(text) if text.isdigit() else text.lower() for text in re.split(r"(\d+)", s)]


def read_hexport_metadata(path: str):
    """Quickly read subject and trial from a .hexport binary file."""
    try:
        with open(path, "rb") as fp:
            header = fp.read(16)
            if len(header) < 16 or header[:4] != b"HEXP":
                return None, None
            version, payload_size, compressed_size = struct.unpack("<III", header[4:16])
            if version != 1 or payload_size == 0 or compressed_size == 0:
                return None, None
            compressed = fp.read(compressed_size)
            if len(compressed) < compressed_size:
                return None, None
        payload = zlib.decompress(compressed)
        if len(payload) < 8:
            return None, None
        row_count, column_count = struct.unpack("<II", payload[:8])
        if row_count == 0:
            return None, None
        offset = 8
        ordered_cols = []
        for _ in range(column_count):
            if offset + 2 > len(payload):
                return None, None
            name_len = payload[offset]
            offset += 1
            dtype = payload[offset]
            offset += 1
            if offset + name_len > len(payload):
                return None, None
            name = payload[offset:offset + name_len].decode("ascii", errors="replace")
            offset += name_len
            ordered_cols.append((name, dtype))

        col_offsets = {}
        for name, dtype in ordered_cols:
            col_offsets[name] = offset
            item_sz = 4 if dtype in (0, 1) else (1 if dtype == 2 else 32)
            offset += item_sz * row_count

        if "subject" not in col_offsets or "trial" not in col_offsets:
            return None, None

        subj_off = col_offsets["subject"]
        trial_off = col_offsets["trial"]
        if subj_off + 32 > len(payload) or trial_off + 32 > len(payload):
            return None, None

        subject = payload[subj_off:subj_off + 32].split(b"\x00")[0].decode("ascii", errors="replace").strip()
        trial = payload[trial_off:trial_off + 32].split(b"\x00")[0].decode("ascii", errors="replace").strip()
        return subject, trial
    except Exception:
        return None, None


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
                subject, trial = read_hexport_metadata(entry.path)
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


def find_frames_dir(export_path_str: str):
    """
    Locate the frames directory for an export file.
    Supports candidate layouts in order of priority:
    1. Pipeline cache layout: <cache_root>/frames/<hash>__<fps>/
    2. Sibling frames layout: <dir>/frames/<hash>__<fps>/
    3. Direct folder layout: <dir>/<hash>__<fps>/
    4. Sibling stem layout: <dir>/frames/<export_stem>/
    5. Cache stem layout: <cache_root>/frames/<export_stem>/
    """
    if not export_path_str:
        return None
    export_path = Path(os.path.expanduser(export_path_str))
    stem = export_path.stem
    parts = stem.split("__")
    if len(parts) >= 2:
        frames_key = f"{parts[0]}__{parts[1]}"
    else:
        frames_key = stem

    candidates = [
        export_path.parent.parent / "frames" / frames_key,
        export_path.parent / "frames" / frames_key,
        export_path.parent / frames_key,
        export_path.parent / "frames" / stem,
        export_path.parent.parent / "frames" / stem,
    ]

    for candidate in candidates:
        if candidate.is_dir():
            return candidate

    return None


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


def sha256_file(path: Path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


_video_hash_cache = {}


def search_video_by_hash(video_hash: str, roots):
    """Last resort: hash every mp4 under `roots` until one matches; results (including misses) are remembered."""
    key = (video_hash, tuple(str(r) for r in roots))
    if key in _video_hash_cache:
        return _video_hash_cache[key]
    found = None
    for root in roots:
        for dirpath, _, names in os.walk(root):
            for name in names:
                if not name.lower().endswith(".mp4"):
                    continue
                candidate = Path(dirpath) / name
                try:
                    if candidate.exists() and sha256_file(candidate) == video_hash:
                        found = candidate
                        break
                except OSError:
                    continue
            if found:
                break
        if found:
            break
    _video_hash_cache[key] = found
    return found


def find_video(export_path_str: str):
    """
    Locate the source mp4 for an export file via the frames directory's .meta.json (`source_video`).
    Falls back to searching next to that path for an mp4 whose SHA-256 matches the hash in the frames key.
    Returns (path, video_hash) or None.
    """
    frames_dir = find_frames_dir(export_path_str)
    if not frames_dir:
        return None
    video_hash = frames_dir.name.split("__")[0]
    source = None
    try:
        with open(frames_dir / ".meta.json", encoding="utf-8") as f:
            meta = json.load(f)
        video_hash = meta.get("video_hash") or video_hash
        source = meta.get("source_video")
    except (OSError, ValueError):
        pass
    if source:
        candidate = Path(os.path.expanduser(source))
        if candidate.exists():
            return candidate, video_hash
    if len(video_hash) != 64:
        return None
    roots = []
    if source:
        parent = Path(os.path.expanduser(source)).parent
        roots += [p for p in (parent, parent.parent) if p.is_dir()]
    found = search_video_by_hash(video_hash, roots)
    return (found, video_hash) if found else None


def handle_get_video(req, out):
    export_path_str = req.get("path") or req.get("export_path", "")
    found = find_video(export_path_str)
    if not found:
        write_json_line(out, {"id": req["id"], "status": "not_found"})
        return
    video, video_hash = found
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
