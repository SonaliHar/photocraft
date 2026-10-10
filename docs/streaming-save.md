# Streamed native saves

Native PSB output uses the background-save service shared by the desktop UI. The service holds
upstream's Arc snapshot and document lock, reports band progress, and records the saved revision
on the original document even when the active tab changes. Same-destination exclusion, worker
completion, and close/quit handling remain those of the upstream save jobs.

The encoder creates and owns a temporary file beside the destination, writes original-depth
channel bands, and flushes the file before calling `SaveCtl.commit()`. This final boundary runs
immediately before atomic replacement. Failure or cancellation before the boundary preserves the
existing destination and removes the temporary. After commit, a late cancellation cannot turn a
successful publication into a reported failure. PSD output that exceeds the PSD size limit also
uses this PSB writer.

Synchronous interactive saves use the same native path encoder. Automation `app.save` uses
upstream's background-save service when available, and retains a synchronous fallback. Both use
only the session's held write-directory capability, including the temporary output and atomic
rename. The background stream checks cancellation before publication through `SaveCtl.commit()`.
Other formats retain their checked byte encoders while sharing the atomic publication boundary.

Streaming PSB saves respect the Maximize Compatibility and Image Previews settings, including
the placeholder merged image when compatibility is disabled and optional PSD thumbnails.

Synthetic regressions exercise precommit rejection, late cancellation, automatic PSD promotion,
tab switching during a native background save, saved identity, and authorized-path rejection.
The integration builds on the save implementation merged in
[PR #2072](https://github.com/storytold/photocraft/pull/2072).
