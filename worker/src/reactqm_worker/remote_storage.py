"""Read-only inventory of a remote Worker store."""
from contextlib import ExitStack, closing
import hashlib
import json
from pathlib import Path
import re
import shutil
import sqlite3
import stat
import time

from .remote_contract import ARTIFACTS, MAX_ARTIFACT, store_lease

ATTEMPT_DIRECTORY = re.compile(
    r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}-[a-z0-9_]+"
)


def inventory(root: Path) -> dict:
    root = root.resolve()
    database = root / "remote.sqlite3"
    if database.is_symlink() or not database.is_file():
        raise ValueError("Expected an existing remote.sqlite3 in the Worker store")

    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=5)) as db:
        # Read both sets from the same SQLite snapshot. Files may change afterward.
        db.execute("BEGIN")
        statuses = dict(db.execute("SELECT status,count(*) FROM jobs GROUP BY status"))
        published = {
            directory for (directory,) in db.execute(
                "SELECT artifact_dir FROM jobs WHERE status='succeeded' AND artifact_dir IS NOT NULL"
            )
        }

    def regular_size(path):
        try:
            metadata = path.lstat()
            if stat.S_ISREG(metadata.st_mode):
                return metadata.st_size
        except FileNotFoundError:
            pass  # The running Worker may remove an attempt while we scan.
        return 0

    database_bytes = sum(regular_size(root / name) for name in (
        "remote.sqlite3", "remote.sqlite3-wal", "remote.sqlite3-shm", "remote.sqlite3-journal",
    ))
    seen = set()
    published_bytes = 0
    unpublished_bytes = 0
    unpublished_directories = 0
    for folder in root.iterdir():
        if not ATTEMPT_DIRECTORY.fullmatch(folder.name) or folder.is_symlink() or not folder.is_dir():
            continue
        try:
            size = sum(regular_size(path) for path in folder.iterdir())
        except FileNotFoundError:
            continue
        if folder.name in published:
            seen.add(folder.name)
            published_bytes += size
        else:
            unpublished_directories += 1
            unpublished_bytes += size

    return {
        "schema_version": 1,
        "jobs_by_status": statuses,
        "database_bytes": database_bytes,
        "published_directories": len(seen),
        "published_bytes": published_bytes,
        "missing_published_directories": len(published - seen),
        "unpublished_directories": unpublished_directories,
        "unpublished_bytes": unpublished_bytes,
    }


def _check_attempt(root, identifier, name, raw_manifest, destination=None, *, allow_missing=False):
    total_bytes = 0
    newest_mtime = float("-inf")
    if (not isinstance(identifier, str) or not isinstance(name, str)
            or not ATTEMPT_DIRECTORY.fullmatch(name) or not name.startswith(identifier + "-")):
        raise ValueError("Invalid published attempt directory")
    attempt = root / name
    try:
        folder_mode = attempt.lstat().st_mode
    except FileNotFoundError:
        if allow_missing:
            return 0, newest_mtime
        raise
    if not stat.S_ISDIR(folder_mode):
        raise ValueError(f"Invalid published attempt directory for {identifier}")
    manifest = json.loads(raw_manifest or "null")
    if (not isinstance(manifest, list) or len(manifest) != len(ARTIFACTS)
            or any(not isinstance(item, dict) or not isinstance(item.get("name"), str)
                   for item in manifest)):
        raise ValueError(f"Invalid artifact manifest for {identifier}")
    entries = {item["name"]: item for item in manifest}
    if set(entries) != set(ARTIFACTS):
        raise ValueError(f"Invalid artifact manifest for {identifier}")
    if destination is not None:
        (destination / name).mkdir(mode=0o700)
    for filename, expected in entries.items():
        path = attempt / filename
        size = expected.get("bytes")
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            if allow_missing:
                continue
            raise
        if (type(size) is not int or not 1 <= size <= MAX_ARTIFACT
                or not stat.S_ISREG(metadata.st_mode) or metadata.st_size != size):
            raise ValueError(f"Artifact size mismatch for {identifier}: {filename}")
        if destination is not None:
            copied = destination / name / filename
            shutil.copyfile(path, copied)
            path = copied
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if path.stat().st_size != size or digest != expected.get("sha256"):
            raise ValueError(f"Artifact checksum mismatch for {identifier}: {filename}")
        total_bytes += size
        newest_mtime = max(newest_mtime, metadata.st_mtime)
    return total_bytes, newest_mtime


def _check_store(root: Path, db, destination: Path | None = None) -> dict:
    if db.execute("PRAGMA quick_check").fetchall() != [("ok",)]:
        raise ValueError("SQLite database failed its integrity check")
    count = db.execute("SELECT count(*) FROM jobs").fetchone()[0]
    published = db.execute(
        "SELECT id,artifact_dir,manifest FROM jobs WHERE status='succeeded' ORDER BY sequence"
    )
    total_bytes = 0
    published_jobs = 0
    for identifier, name, raw_manifest in published:
        size, _ = _check_attempt(root, identifier, name, raw_manifest, destination)
        total_bytes += size
        published_jobs += 1

    return {"schema_version": 1, "jobs": count, "published_jobs": published_jobs,
            "artifact_bytes": total_bytes}


def verify_archive(root: Path) -> dict:
    """Check a stopped archive without starting its dispatcher."""
    database = root.resolve() / "remote.sqlite3"
    if database.is_symlink() or not database.is_file():
        raise ValueError("Expected an existing remote.sqlite3 in the archive")
    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=5)) as db:
        db.execute("BEGIN")
        return _check_store(database.parent, db)


def archive(root: Path, destination: Path) -> dict:
    """Copy a stopped Worker store into a new, directly usable directory."""
    import tempfile

    root = root.resolve()
    destination = destination.parent.resolve() / destination.name
    if destination == root or root in destination.parents:
        raise ValueError("The archive must be outside the source store")
    database = root / "remote.sqlite3"
    if database.is_symlink() or not database.is_file():
        raise ValueError("Expected an existing remote.sqlite3 in the Worker store")

    with store_lease(root):
        destination.mkdir(mode=0o700)
        try:
            with tempfile.TemporaryDirectory(prefix=".remote-archive-", dir=destination.parent) as staging:
                folder = Path(staging) / "store"
                folder.mkdir(mode=0o700)
                with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as source:
                    with closing(sqlite3.connect(folder / database.name)) as backup:
                        source.backup(backup)
                        report = _check_store(root, backup, folder)

                folder.replace(destination)
        except BaseException:
            if destination.is_dir() and not any(destination.iterdir()):
                destination.rmdir()
            raise

    return report


def _archive_matches(db, identifier, digest, manifest):
    row = db.execute(
        "SELECT digest,manifest FROM jobs WHERE id=? AND status='succeeded'", (identifier,)
    ).fetchone()
    return row == (digest, manifest)


def retention_plan(root: Path, archive_root: Path, retain_days: int) -> dict:
    """Report eligible jobs from offline stores without changing either store."""
    if retain_days < 1:
        raise ValueError("Retention must be at least one day")
    source_db = root.resolve() / "remote.sqlite3"
    archive_db = archive_root.resolve() / "remote.sqlite3"
    for database in (source_db, archive_db):
        if database.is_symlink() or not database.is_file():
            raise ValueError("Both stores must contain an existing remote.sqlite3")
    if source_db.samefile(archive_db):
        raise ValueError("The archive must be a separate store")

    now = time.time()
    candidates = []
    with closing(sqlite3.connect(source_db.as_uri() + "?mode=ro", uri=True)) as source:
        with closing(sqlite3.connect(archive_db.as_uri() + "?mode=ro", uri=True)) as backup:
            source.execute("BEGIN")
            backup.execute("BEGIN")
            _check_store(archive_db.parent, backup)
            for identifier, digest, directory, manifest in source.execute(
                "SELECT id,digest,artifact_dir,manifest FROM jobs WHERE status='succeeded' ORDER BY sequence"
            ):
                if not _archive_matches(backup, identifier, digest, manifest):
                    continue
                size, newest_mtime = _check_attempt(source_db.parent, identifier, directory, manifest)
                age_days = (now - newest_mtime) / 86400
                if age_days >= retain_days:
                    candidates.append({"job_id": identifier, "artifact_bytes": size,
                                       "age_days": int(age_days)})

    return {"schema_version": 1, "retention_days": retain_days, "candidates": candidates,
            "candidate_bytes": sum(item["artifact_bytes"] for item in candidates)}


def apply_retention(root: Path, archive_root: Path, retain_days: int, job_id: str) -> dict:
    """Recheck and remove one explicitly selected, archived job's source artifacts."""
    import uuid

    if str(uuid.UUID(job_id)) != job_id:
        raise ValueError("Invalid job ID")
    root, archive_root = root.resolve(), archive_root.resolve()
    if root == archive_root:
        raise ValueError("The archive must be a separate store")

    with ExitStack() as stack:
        for store in (root, archive_root):
            if not (store / "remote.sqlite3").is_file():
                raise ValueError("Both stores must contain an existing remote.sqlite3")
            stack.enter_context(store_lease(store))

        source_db = root / "remote.sqlite3"
        backup_db = archive_root / "remote.sqlite3"
        if source_db.is_symlink() or backup_db.is_symlink() or source_db.samefile(backup_db):
            raise ValueError("Expected separate Worker databases")
        if retain_days < 1:
            raise ValueError("Retention must be at least one day")

        with closing(sqlite3.connect(source_db.as_uri() + "?mode=rw", uri=True)) as db, db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute(
                "SELECT digest,manifest,artifact_dir,status FROM jobs WHERE id=?", (job_id,)
            ).fetchone()
            if row is None or row[3] not in ("succeeded", "archived"):
                raise ValueError("The selected job is not eligible for retention cleanup")
            digest, manifest, directory, status = row
            with closing(sqlite3.connect(backup_db.as_uri() + "?mode=ro", uri=True)) as backup:
                backup.execute("BEGIN")
                _check_store(archive_root, backup)
                if not _archive_matches(backup, job_id, digest, manifest):
                    raise ValueError("The selected job has no matching archived copy")
            size, newest = _check_attempt(root, job_id, directory, manifest,
                                          allow_missing=status == "archived")
            if time.time() - newest < retain_days * 86400:
                raise ValueError("The selected job is within the retention period")
            db.execute("UPDATE jobs SET status='archived' WHERE id=?", (job_id,))

        # Commit first. A retry verifies the archive and all surviving files again.
        attempt = root / directory
        for name in ARTIFACTS:
            (attempt / name).unlink(missing_ok=True)
        if attempt.exists() and not any(attempt.iterdir()):
            attempt.rmdir()

    return {"schema_version": 1, "job_id": job_id, "status": "archived", "removed_bytes": size}
