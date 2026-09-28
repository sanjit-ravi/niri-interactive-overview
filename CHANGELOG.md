# Changes

## Initial independent extraction — 2026-09-28

Base: Niri v26.04, `8ed0da44d974c32c6877d2f4630c314da0717ecb`.

- Extract the existing mouse-first fit overview, camera transitions, preview/window dragging, workspace badges/reordering, close controls and optional interactive application previews.
- Preserve opt-in pointer routing, deferred single clicks, activation double clicks, ordinary client dragging, compositor-modifier preview dragging and popup/grab safeguards.
- Make held desktop canvas navigation configurable with `overview { canvas-button CODE; }`, disabled by default; preserve normal button dispatch when disabled and give explicit bindings priority.
- Exclude automatic single-window full-width/maximized-to-edges behavior and application-specific preview branding. Retain generic icon lookup and canvas-map animations.
- Add protocol coverage for button remapping, release drainage across configuration changes, compositor binding precedence and native grabbed popup dismissal.
- Correct two input ownership defects found during extraction review: cancel canvas navigation when overview/modal/grab ownership changes without hiding the fit cursor, and retain cancelled preview grabs until swallowed secondary-button releases are drained.
- Correct interrupted fit-overview gesture cleanup: cancelling the underlying workspace switch now removes its obsolete empty workspace. Found by the upstream randomized layout invariant test and retained as a deterministic regression.
- Provide standalone configuration, side-by-side install and rollback instructions, privacy-conscious contribution guidance and build/test CI.

These are extraction and verification changes performed on this date. Earlier local development was uncommitted; no historical feature commits were reconstructed or backdated. Upstream Git history and licensing remain intact.
