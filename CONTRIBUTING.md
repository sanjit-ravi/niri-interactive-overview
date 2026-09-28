# Contributing to Niri Interactive Overview

This is an independent experimental derivative. Open derivative-specific issues and pull requests in this repository. Do not represent this project as an official Niri release.

## Reproductions and privacy

Include the fork commit, application ID, native Wayland versus XWayland status, overview mode, output scale, exact gesture and expected/observed result. Use the smallest sanitized configuration that reproduces the issue. Do not upload full personal configurations, credentials, notifications, window titles or recordings without reviewing them.

Distinguish virtual-pointer tests, nested render checks and physical-device testing. A test client named after an application is not evidence that the actual application was exercised.

## Code changes

Preserve pointer ownership, balanced button edges, compositor binding priority, popup/keyboard isolation and cancellation on target destruction or overview closure. Prefer regression tests using the existing Wayland client fixture for routing changes and layout tests for transforms/geometry. Do not replace behavior tests with source-text assertions or Debug snapshots.

Follow the build/test commands in README.md. Run broader tests after input/layout changes. Keep upstream licensing and attribution. Explain AI assistance and independently verify its output; the submitter remains responsible for correctness. Do not fabricate test evidence or development history.

## Upstream boundary

Niri's current contribution policy prohibits LLM-created code, issues, comments and other contributions. Read the [current upstream policy](https://github.com/niri-wm/niri/blob/main/CONTRIBUTING.md) before any upstream interaction. Do not forward generated material upstream. The original policy remains available in the preserved upstream Git history; this file governs only this derivative.
