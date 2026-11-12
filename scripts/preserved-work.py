#!/usr/bin/env python3
"""List, restore, and verify preserved work without the original worktrees.

Use --help and each subcommand's --help. No command pushes, merges into the current
checkout, deletes old worktrees, or treats an archive snapshot as verified code.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import sqlite3
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
INDEX = ROOT / "research/v2-restoration/rounds-1-2/worktree-snapshots.json"


def sha_file(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def git(cwd, *args):
    result = subprocess.run(["git", "-C", str(cwd), *args], text=True,
                            capture_output=True, check=True)
    return result.stdout.strip()


def db_open(archive):
    path = archive.resolve() / "evidence.sqlite3"
    if not path.is_file():
        raise ValueError("missing evidence.sqlite3 under " + str(archive))
    return sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)


def restore_work(args):
    records = json.loads(INDEX.read_text())
    record = next((r for r in records if r["id"] == args.id), None)
    if record is None:
        raise ValueError("unknown snapshot; use list-work")
    dest = args.destination.absolute()
    if dest.exists() or dest.is_symlink():
        raise ValueError("destination must not already exist")
    base = record["mergeBaseWithMainBefore"]
    git(ROOT, "cat-file", "-e", base + "^{commit}")
    patch = ROOT / record["patch"] if record.get("patch") else None
    if patch is not None and sha_file(patch) != record["patchSha256"]:
        raise ValueError("preserved patch hash mismatch")
    dest.mkdir(parents=True)
    git(dest, "init", "--quiet")
    git(dest, "fetch", "--quiet", "--no-tags", str(ROOT), base)
    git(dest, "checkout", "--quiet", "--detach", "FETCH_HEAD")
    if patch is not None:
        git(dest, "apply", "--index", "--binary", str(patch))
    actual = git(dest, "write-tree")
    if actual != record["snapshotTree"]:
        raise ValueError("restored tree mismatch; preserve destination for diagnosis")
    print(json.dumps({"id": args.id, "destination": str(dest), "tree": actual,
                      "treeVerified": True, "acceptedIntoMain": False,
                      "note": "Original base is checked out; snapshot changes are staged. Review before committing."}, indent=2))


def extract(args):
    dest = args.destination.absolute()
    if dest.exists() or dest.is_symlink():
        raise ValueError("destination must not already exist")
    prefix = args.prefix.strip("/")
    if ".." in PurePosixPath(prefix).parts:
        raise ValueError("invalid prefix")
    with db_open(args.archive) as db:
        if db.execute("SELECT 1 FROM roots WHERE id=?", (args.root,)).fetchone() is None:
            raise ValueError("unknown evidence root; use list-evidence")
        # Exact prefix semantics, not SQL wildcard matching.
        names = db.execute("SELECT path,sha256,kind,mode FROM files WHERE root=? ORDER BY path", (args.root,))
        dest.mkdir(parents=True)
        count, links = 0, []
        for path, digest, kind, mode in names:
            if prefix and path != prefix and not path.startswith(prefix + "/"):
                continue
            relative = PurePosixPath(path)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("unsafe stored path " + path)
            row = db.execute("SELECT size,compressed FROM objects WHERE sha256=?", (digest,)).fetchone()
            if row is None:
                raise ValueError("missing object " + digest)
            data = zlib.decompress(row[1])
            if len(data) != row[0] or hashlib.sha256(data).hexdigest() != digest:
                raise ValueError("corrupt object " + digest)
            if kind == "symlink":
                # Never recreate a link into a deleted worktree or outside the destination.
                links.append({"path": path, "originalTarget": os.fsdecode(data)})
            else:
                target = dest.joinpath(*relative.parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open("xb") as stream:
                    stream.write(data)
                target.chmod(mode & 0o777)
            count += 1
        if not count:
            raise ValueError("prefix matched no records; no success was inferred")
        with (dest / "PRESERVED-SYMLINKS.json").open("x") as manifest:
            manifest.write(json.dumps(links, indent=2) + "\n")
    print(json.dumps({"recordsVerified": count, "symlinksRecordedNotFollowed": len(links),
                      "destination": str(dest)}, indent=2))


def verify(args):
    archive = args.archive.resolve()
    for name in ["evidence-database.json", "bundle.json",
                 "private-project-sessions.json", "binary-fixtures-supplement.json",
                 "final-git-bundle.json", "final-source-archive.json"]:
        if name not in {"evidence-database.json", "bundle.json"} and not (archive / "manifests" / name).exists():
            continue
        record = json.loads((archive / "manifests" / name).read_text())
        path = archive / record.get("path", record.get("file"))
        if path.stat().st_size != record["bytes"] or sha_file(path) != record["sha256"]:
            raise ValueError("archive hash mismatch: " + str(path))
    git(ROOT, "bundle", "verify", str(archive / "all-refs-preserved.bundle"))
    with db_open(archive) as db:
        if db.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise ValueError("database integrity failure")
        missing = db.execute("SELECT count(*) FROM files f LEFT JOIN objects o ON f.sha256=o.sha256 WHERE o.sha256 IS NULL").fetchone()[0]
        if missing:
            raise ValueError("evidence paths have missing content")
        objects = 0
        for digest, size, packed in db.execute("SELECT sha256,size,compressed FROM objects"):
            data = zlib.decompress(packed)
            if len(data) != size or hashlib.sha256(data).hexdigest() != digest:
                raise ValueError("object checksum failure " + digest)
            objects += 1
        files = db.execute("SELECT count(*) FROM files").fetchone()[0]
    print(json.dumps({"archive": str(archive), "verified": True,
                      "objects": objects, "fileRecords": files}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("list-work", help="list source snapshots and their qualification")
    restore = sub.add_parser("restore-work", help="restore exact source into a NEW independent local Git repository")
    restore.add_argument("id")
    restore.add_argument("--destination", type=Path, required=True)
    ls = sub.add_parser("list-evidence", help="list private/raw evidence roots")
    ls.add_argument("--archive", type=Path, required=True)
    ex = sub.add_parser("extract-evidence", help="extract verified evidence into a NEW directory")
    ex.add_argument("--archive", type=Path, required=True)
    ex.add_argument("--root", required=True)
    ex.add_argument("--prefix", default="")
    ex.add_argument("--destination", type=Path, required=True)
    ve = sub.add_parser("verify", help="verify bundle, database and every compressed evidence object")
    ve.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "list-work":
            for r in json.loads(INDEX.read_text()):
                print(r["id"], r["snapshotCommit"], r["status"], sep="\t")
        elif args.action == "restore-work":
            restore_work(args)
        elif args.action == "list-evidence":
            with db_open(args.archive) as db:
                for row in db.execute("SELECT r.id,r.role,count(f.path) FROM roots r LEFT JOIN files f ON f.root=r.id GROUP BY r.id ORDER BY r.id"):
                    print(*row, sep="\t")
        elif args.action == "extract-evidence":
            extract(args)
        else:
            verify(args)
    except (ValueError, OSError, sqlite3.Error, subprocess.CalledProcessError) as exc:
        print("preserved-work: " + str(exc), file=sys.stderr)
        if isinstance(exc, subprocess.CalledProcessError) and exc.stderr:
            print(exc.stderr, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
