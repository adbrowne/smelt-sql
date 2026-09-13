"""Job-native probe for the Databricks Asset Bundle's `volume_probe` task
(docs/outcomes/20260912-databricks-dogfood-spine/phases/11c-plan.md).

`.smelt/`'s run-state ledger relies on two filesystem guarantees on whatever
directory it lives in: an advisory `flock` on `.smelt/lock` serialising
concurrent runs, and `os.replace()` renaming a temp manifest over the live
one atomically so a reader never observes a half-written file. Both are
POSIX filesystem guarantees; a Unity Catalog Volume is FUSE-mounted, and FUSE
layers are not obligated to honour either. This job measures both directly
against the *deployed* Volume path rather than assuming — the result feeds
the phase's conditional spec delta (a Known Divergence in
docs/specs/run_state.md if either guarantee does not hold).

Prints one JSON object with a verdict per probe so the wrapping shell script
can capture it verbatim into phases/11c-volume-probe.md.
"""

import errno
import json
import multiprocessing
import os
import sys
import tempfile


def probe_flock_advisory(lock_path):
    """A concurrent process trying to flock the same file must block or fail
    while the first holder has it open, and succeed once released."""
    import fcntl

    def child(lock_path, result_queue):
        try:
            with open(lock_path, "a+") as fh:
                fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
                result_queue.put("acquired")
        except OSError as e:
            if e.errno in (errno.EACCES, errno.EAGAIN):
                result_queue.put("blocked")
            else:
                result_queue.put(f"error:{e}")

    holder = open(lock_path, "a+")
    try:
        fcntl.flock(holder, fcntl.LOCK_EX)

        result_queue = multiprocessing.Queue()
        proc = multiprocessing.Process(target=child, args=(lock_path, result_queue))
        proc.start()
        proc.join(timeout=15)
        second_holder_result = result_queue.get(timeout=1) if not result_queue.empty() else "timeout"

        return {
            "honoured": second_holder_result == "blocked",
            "second_holder_result": second_holder_result,
        }
    finally:
        fcntl.flock(holder, fcntl.LOCK_UN)
        holder.close()


def probe_rename_atomicity(target_dir):
    """os.replace() must land the new content in full or not at all — never
    a reader observing a truncated or missing file mid-rename."""
    live_path = os.path.join(target_dir, "_probe_manifest.json")
    with open(live_path, "w") as fh:
        fh.write(json.dumps({"version": 1}))

    tmp_fd, tmp_path = tempfile.mkstemp(dir=target_dir, suffix=".tmp")
    with os.fdopen(tmp_fd, "w") as fh:
        fh.write(json.dumps({"version": 2}))
        fh.flush()
        os.fsync(fh.fileno())

    try:
        os.replace(tmp_path, live_path)
        with open(live_path) as fh:
            content = json.load(fh)
        landed = content == {"version": 2}
    except OSError as e:
        return {"honoured": False, "error": str(e)}
    finally:
        if os.path.exists(live_path):
            os.remove(live_path)
        if os.path.exists(tmp_path):
            os.remove(tmp_path)

    return {"honoured": landed}


def probe_fsync(target_dir):
    path = os.path.join(target_dir, "_probe_fsync.tmp")
    try:
        with open(path, "wb") as fh:
            fh.write(b"probe")
            fh.flush()
            os.fsync(fh.fileno())
        ok = True
        error = None
    except OSError as e:
        ok = False
        error = str(e)
    finally:
        if os.path.exists(path):
            os.remove(path)
    return {"honoured": ok, "error": error}


def main():
    if len(sys.argv) < 2 or not sys.argv[1]:
        raise SystemExit("volume_probe.py requires the Volume-resident project directory")
    project_dir = sys.argv[1]
    smelt_dir = os.path.join(project_dir, ".smelt")
    os.makedirs(smelt_dir, exist_ok=True)
    lock_path = os.path.join(smelt_dir, "lock")

    verdict = {
        "flock_advisory": probe_flock_advisory(lock_path),
        "rename_atomicity": probe_rename_atomicity(smelt_dir),
        "fsync": probe_fsync(smelt_dir),
    }
    print(json.dumps(verdict))


if __name__ == "__main__":
    main()
