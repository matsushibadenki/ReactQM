"""Shared file contract for the loopback client, dispatcher and offline tools."""
from contextlib import contextmanager

MAX_REQUEST = 65536
MAX_ARTIFACT = 64 * 1024 * 1024
ARTIFACTS = ("input.json", "result.json", "result.h5")


@contextmanager
def store_lease(root):
    import fcntl

    with (root / "remote.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError("Remote Worker store is in use") from error
        yield
