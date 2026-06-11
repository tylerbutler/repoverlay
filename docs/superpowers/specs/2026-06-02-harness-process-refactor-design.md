# Harness Process Refactor Design

## Purpose

`repoverlay copilot --profile` currently owns low-level process-group, signal, and terminal
foreground details directly in the Copilot command handler. That code works, but it makes the
profile lifecycle harder to reason about because harness execution mechanics are mixed with apply
and cleanup orchestration.

Refactor harness execution behind a small internal abstraction that uses external crates for the
parts they handle well:

- `process-wrap` for managed child process groups and cross-platform process wrapping.
- `nix` for typed Unix terminal and signal APIs where shell-like terminal handoff is still needed.

The refactor should preserve current CLI behavior.

## Goals

- Keep `src/cli/commands/copilot.rs` focused on profile lifecycle: apply, run, cleanup, and exit-code
  propagation.
- Replace direct process-group setup and group termination logic with `process-wrap`.
- Replace raw Unix `libc` calls in the Copilot process path with `nix` wrappers.
- Keep profile lock ownership and stale-lock recovery in profile/state code.
- Avoid introducing a PTY or terminal emulator model.
- Preserve existing integration test behavior; add only focused unit tests for new seams.

## Non-goals

- Do not change the `repoverlay copilot --profile` user interface.
- Do not change profile apply/remove semantics.
- Do not run Copilot under a pseudo-terminal.
- Do not rewrite unrelated command execution paths.
- Do not add new async/runtime dependencies.

## Dependencies

Add `process-wrap` with its std frontend and process-management wrappers. The implementation should
enable only the features needed by the chosen API, expected to include:

- `std`
- `process-group`
- `job-object`
- `kill-on-drop`

Add a direct `nix` dependency for typed Unix primitives. Expected feature set:

- `process`
- `signal`
- `term`

`process-wrap` may depend on `nix` internally for Unix support, but `repoverlay` should depend on
`nix` directly because terminal foreground handoff and PID liveness checks are application-owned
logic.

## Architecture

Introduce an internal `harness_process` module.

`copilot.rs` should:

1. Build the harness command through the existing profile applicator.
2. Set the target working directory.
3. Pass the command to `HarnessProcess::spawn`.
4. Register the child with the existing interrupt machinery.
5. Wait using the same interruption-aware loop.
6. Clean up the profile and propagate the harness exit code.

`harness_process` should own the process mechanics:

- Convert `std::process::Command` into `process_wrap::std::CommandWrap`.
- On Unix, wrap with `ProcessGroup::leader()` so the harness becomes the process-group leader.
- On Windows, wrap with `JobObject` when supported by `process-wrap`.
- Expose a narrow API needed by callers:
  - `id()`
  - `try_wait()`
  - `terminate()`
  - any small helper needed to determine the child process group for terminal handoff

This keeps crate-specific types from leaking into profile or CLI lifecycle code.

## Terminal Foreground Handling

Keep terminal foreground management separate from process spawning.

`process-wrap` creates/manages process groups, but it does not make the child process group the
foreground owner of the controlling terminal. Interactive harnesses still need foreground ownership
while they run.

The existing `TerminalForeground` guard should move into the new process module or a nearby private
submodule and be rewritten with `nix`:

- Detect whether stdin is a terminal.
- Save the current foreground process group with `tcgetpgrp`.
- Set the child process group as foreground with `tcsetpgrp`.
- Restore the original foreground process group before profile cleanup.
- Keep handoff best-effort: if stdin is not a TTY or terminal APIs fail, continue without handoff.

Any temporary signal disposition needed to avoid `SIGTTOU` should use `nix::sys::signal` rather than
raw `libc`.

## Lock and Liveness Handling

Keep lock state in `src/profile.rs`.

Profile locks are system-managed metadata about profile application, not harness process management.
The existing PID lock file and stale-lock recovery remain the right model.

The Unix implementation of PID liveness may be rewritten with:

- `nix::sys::signal::kill(Pid, None)`
- `ESRCH` means stale.
- `EPERM` means live.
- PID `0` remains invalid for a single lock owner and should be treated as stale.

## Error Handling

Spawn failure must still trigger profile cleanup before returning an error.

Termination should prefer the managed group/job termination behavior from `process-wrap`. If that
fails, the code should fall back to killing the direct child where the API supports it. Cleanup errors
remain surfaced as failures after harness exit.

Terminal handoff failures should not block non-interactive use. They should preserve current
best-effort behavior rather than preventing the harness from running.

## Testing

Existing integration tests should remain the primary behavioral coverage:

- `cargo test copilot_profile`
- `cargo test profile_`

Expected test changes:

- Keep CLI assertions and cleanup checks unchanged where possible.
- Add unit tests around the new internal process wrapper seams where they are deterministic.
- Keep no-TTY terminal handoff tests.
- Keep stale-lock and live-lock tests.

Do not add a PTY dependency for this refactor. A true interactive terminal regression test can be
considered later if real-world Copilot TTY behavior remains uncertain.
