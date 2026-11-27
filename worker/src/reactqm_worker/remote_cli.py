"""CLI adapters for the remote Worker and offline store operations."""
import json
import sqlite3
import sys

from .remote_storage import apply_retention, archive, inventory, retention_plan, verify_archive

# CLI argument order matches each operation's public function signature.
_STORE_OPERATIONS = {
    "remote-storage": (inventory, ("output_dir",), "Remote storage"),
    "remote-archive": (archive, ("output_dir", "archive_dir"), "Remote archive"),
    "remote-verify-archive": (verify_archive, ("archive_dir",), "Remote archive"),
    "remote-retention-plan": (retention_plan, ("output_dir", "archive_dir", "retain_days"), "Remote retention"),
    "remote-retention-apply": (apply_retention, ("output_dir", "archive_dir", "retain_days", "job_id"), "Remote retention"),
}
COMMANDS = ("remote-serve", "remote-run", "remote-cancel", *_STORE_OPERATIONS)


def dispatch(parser, args):
    if args.command in _STORE_OPERATIONS:
        operation, required, label = _STORE_OPERATIONS[args.command]
        values = [getattr(args, key) for key in required]
        if None in values:
            flags = ", ".join("--" + key.replace("_", "-") for key in required)
            parser.error(f"{args.command} requires {flags}")
        try:
            print(json.dumps(operation(*values), allow_nan=False))
        except (ValueError, OSError, sqlite3.Error) as error:
            parser.exit(1, f"{label}: {error}\n")
        return
    if args.command == "remote-cancel":
        if args.job_id is None or not 1 <= args.port <= 65535:
            parser.error("remote-cancel requires --job-id and a valid port")
        from .remote_client import cancel, RemoteJobMissing
        try:
            state = cancel(args.job_id, args.port)
            if args.wait:
                import time
                deadline = time.monotonic() + 15
                while state["status"] == "cancelling" and time.monotonic() < deadline:
                    time.sleep(0.5)
                    state = cancel(args.job_id, args.port)
                if state["status"] == "cancelling":
                    parser.exit(1, "Remote Worker: cancellation still pending\n")
            print(json.dumps(state, allow_nan=False))
            if state["status"] not in ("cancelling", "cancelled"):
                parser.exit(3, "Remote Worker: the job was already terminal\n")
        except RemoteJobMissing as error:
            parser.exit(4, f"Remote Worker: {error}\n")
        except (ValueError, OSError) as error:
            parser.exit(1, f"Remote Worker: {error}\n")
        return
    if args.command == "remote-run":
        if args.input is None or args.output_dir is None or not 1 <= args.port <= 65535:
            parser.error("remote-run requires --input, --output-dir and a valid port")
        from .remote_client import run
        try:
            if args.phase_stream:
                def emit_phase(phase):
                    sys.stdout.buffer.write(bytes([phase]))
                    sys.stdout.buffer.flush()

                run(args.input, args.output_dir, args.port, on_phase=emit_phase)
            else:
                print(json.dumps(run(args.input, args.output_dir, args.port), allow_nan=False))
        except (ValueError, OSError) as error:
            parser.exit(1, f"Remote Worker: {error}\n")
        return
    if args.command == "remote-serve":
        if args.output_dir is None or not 1 <= args.port <= 65535:
            parser.error("remote-serve requires --output-dir and a valid port")
        from .remote import serve
        serve(args.output_dir, args.port)
        return
