#!/usr/bin/env bash
#
# PROTOTYPE — a `cargo` shim that serialises heavy builds across worktrees.
#
# NOT ACTIVE. Nothing puts this on PATH. To try it:
#
#   mkdir -p ~/.local/smelt-shim
#   ln -sf "$PWD/.claude/scripts/cargo-queue-shim.sh" ~/.local/smelt-shim/cargo
#   export PATH="$HOME/.local/smelt-shim:$PATH"     # or add to .mise.local.toml
#
# To back it out: remove the symlink. Nothing else changes.
#
# WHY
# ---
# Several agent sessions work in several worktrees at once. Each runs its own
# cargo, and cargo defaults `-j` to the core count, so the machine is
# oversubscribed N-fold. `CARGO_BUILD_JOBS` in mise.toml bounds that fan-out,
# but it cannot give a single build the whole machine when it is in fact alone,
# and it does not stop three builds thrashing cache and memory simultaneously.
#
# This shim adds the missing half: heavy cargo invocations queue on one
# machine-wide lock. Serialised, three 200s builds finish at 200/400/600s.
# Run concurrently, all three finish around 1000s. Same total work; the queue
# just stops them fighting.
#
# DESIGN CONSTRAINTS (each one is a failure mode this has to avoid)
# -----------------------------------------------------------------
# * Fail open. If anything is wrong — no flock, no real cargo, unwritable lock
#   dir — run the build anyway. A broken shim in front of every build in every
#   worktree is far worse than an unserialised build.
# * Never in CI. `CI` is set on every runner; the shim exits to real cargo
#   immediately. Committing a local build tweak that reached CI is exactly what
#   forced the #150 revert of the mold config.
# * Reentrant. A build script, a test, or `cargo run` invoking cargo again must
#   not deadlock behind the lock its own parent holds.
# * Bounded wait. An agent's Bash tool times out (600s cap), so a session queued
#   behind two builds would die waiting. After SMELT_BUILD_LOCK_WAIT seconds the
#   shim gives up on the lock and builds anyway at the reduced job count —
#   degraded, but always making progress.
# * Cheap subcommands never queue. `cargo metadata` / `fmt` / `tree` are used
#   interactively and must not block behind a 200s build.

set -u

# ---- resolve the real cargo -------------------------------------------------
# ~/.cargo/bin/cargo is the rustup proxy: a stable path that respects
# rust-toolchain.toml and RUSTUP_TOOLCHAIN, so the shim cannot change which
# toolchain is used.
REAL_CARGO="${SMELT_REAL_CARGO:-$HOME/.cargo/bin/cargo}"
if [ ! -x "$REAL_CARGO" ]; then
  # Fail open: find any cargo that is not this script.
  self=$(readlink -f "$0" 2>/dev/null || echo "$0")
  while IFS= read -r c; do
    [ "$(readlink -f "$c" 2>/dev/null || echo "$c")" = "$self" ] && continue
    REAL_CARGO="$c"; break
  done < <(command -v -a cargo 2>/dev/null)
fi
[ -x "$REAL_CARGO" ] || { echo "cargo-queue-shim: no real cargo found" >&2; exit 127; }

exec_real() { exec "$REAL_CARGO" "$@"; }

# ---- bail-out paths ---------------------------------------------------------
[ -n "${CI:-}" ] && exec_real "$@"                      # never queue in CI
[ -n "${SMELT_CARGO_LOCK_HELD:-}" ] && exec_real "$@"   # reentrant call
command -v flock >/dev/null 2>&1 || exec_real "$@"      # no flock, fail open

# First non-flag argument is the subcommand.
sub=""
for a in "$@"; do
  case "$a" in
    -*) continue ;;
    *) sub="$a"; break ;;
  esac
done

# Only these actually compile. Everything else (fmt, metadata, tree, pkgid,
# locate-project, search, --version, help) runs immediately.
case "$sub" in
  build|b|check|c|test|t|clippy|run|r|bench|doc|rustc|nextest|miri) ;;
  *) exec_real "$@" ;;
esac

# ---- acquire the lock -------------------------------------------------------
LOCK_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/smelt"
LOCK="${SMELT_BUILD_LOCK_FILE:-$LOCK_DIR/build.lock}"
WAIT="${SMELT_BUILD_LOCK_WAIT:-420}"
FULL_JOBS="${SMELT_BUILD_FULL_JOBS:-$(nproc 2>/dev/null || echo 8)}"

mkdir -p "$LOCK_DIR" 2>/dev/null || exec_real "$@"

# NOTE: the braces matter. `exec 9>"$LOCK" 2>/dev/null` would apply BOTH
# redirections permanently to this shell, silencing stderr for the rest of the
# script — and cargo inherits it, so every compiler error would be swallowed.
# Grouping scopes the 2>/dev/null to the group while fd 9 persists past it.
{ exec 9>"$LOCK"; } 2>/dev/null || exec_real "$@"

start=$(date +%s)
if flock -n 9 2>/dev/null; then
  : # uncontended
else
  holder=$(cat "$LOCK" 2>/dev/null | head -1)
  echo "cargo-queue-shim: waiting for build lock (held by: ${holder:-unknown}); up to ${WAIT}s" >&2
  if ! flock -w "$WAIT" 9 2>/dev/null; then
    # Degraded path: never block forever, never oversubscribe either.
    echo "cargo-queue-shim: lock wait exceeded ${WAIT}s — building at CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-12}" >&2
    export SMELT_CARGO_LOCK_HELD=1
    exec_real "$@"
  fi
  echo "cargo-queue-shim: acquired after $(( $(date +%s) - start ))s" >&2
fi

# Record who holds it, for the message above. The fd stays open across exec, so
# the lock is held for exactly as long as cargo runs.
echo "pid=$$ wt=$(git rev-parse --show-toplevel 2>/dev/null || pwd) sub=$sub $(date -Is)" >&9 2>/dev/null

# We own the machine for this build, so use all of it.
export CARGO_BUILD_JOBS="$FULL_JOBS"
export SMELT_CARGO_LOCK_HELD=1
exec_real "$@"
