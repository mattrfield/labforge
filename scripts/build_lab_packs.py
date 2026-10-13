#!/usr/bin/env python3
"""Builds lab packs and the lab index that the launcher's "Update labs" feature reads.

    python scripts/build_lab_packs.py --out dist-labs --base-url https://.../releases/download/labs

For every `labs/<id>/lab.yaml` of the free tier this writes `<id>-<version>.tar.gz` (one top-level
`<id>/` folder, reproducible bytes) and a single `index.json` listing each pack's URL, size and
SHA-256. The index is signed separately (see .github/workflows/labs.yml). Standard library only.
"""
import argparse
import gzip
import hashlib
import io
import json
import re
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
SKIP_NAMES = {".DS_Store", "Thumbs.db"}


def scalar(text: str, key: str):
    """Top-level `key: value` from a lab.yaml, without needing a YAML parser."""
    m = re.search(rf"^{key}:[ \t]*(.*?)[ \t]*$", text, re.M)
    if not m:
        return None
    value = m.group(1)
    if value in ("", ">", "|", ">-", "|-"):
        return None
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        value = value[1:-1]
    return value


def build_pack(lab_dir: Path, lab_id: str) -> bytes:
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
            for path in sorted([lab_dir, *lab_dir.rglob("*")]):
                if path.name in SKIP_NAMES or path.is_symlink():
                    continue
                rel = path.relative_to(lab_dir)
                info = tar.gettarinfo(str(path), arcname=str(Path(lab_id) / rel).replace("\\", "/"))
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 0
                if path.is_file():
                    info.mode = 0o755 if info.mode & 0o111 else 0o644
                    with open(path, "rb") as f:
                        tar.addfile(info, f)
                else:
                    info.mode = 0o755
                    tar.addfile(info)
    return buf.getvalue()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="dist-labs")
    ap.add_argument("--base-url", required=True, help="where the packs will be downloadable from")
    ap.add_argument("--labs-dir", default=str(ROOT / "labs"))
    args = ap.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    entries = []
    for lab_yaml in sorted(Path(args.labs_dir).glob("*/lab.yaml")):
        text = lab_yaml.read_text(encoding="utf-8")
        lab_id, version = scalar(text, "id"), scalar(text, "version") or "1.0.0"
        tier = scalar(text, "tier") or "free"
        if lab_id != lab_yaml.parent.name:
            sys.exit(f"{lab_yaml}: id `{lab_id}` must match the folder name")
        if not SEMVER.match(version):
            sys.exit(f"{lab_yaml}: version `{version}` must look like 1.2.0")
        if tier != "free":
            print(f"skip {lab_id}: tier {tier} is not published to the public index")
            continue
        data = build_pack(lab_yaml.parent, lab_id)
        name = f"{lab_id}-{version}.tar.gz"
        (out / name).write_bytes(data)
        entry = {
            "id": lab_id,
            "name": scalar(text, "name") or lab_id,
            "version": version,
            "url": f"{args.base_url.rstrip('/')}/{name}",
            "sha256": hashlib.sha256(data).hexdigest(),
            "size": len(data),
            "tier": tier,
            "blurb": scalar(text, "blurb") or "",
        }
        if scalar(text, "min_app_version"):
            entry["min_app_version"] = scalar(text, "min_app_version")
        entries.append(entry)
        print(f"packed {name} ({len(data) // 1024} KiB)")

    index = {"schema": 1, "labs": entries}
    (out / "index.json").write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote index.json with {len(entries)} labs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
