# AssetCraft — Roadmap

This document tracks where AssetCraft is going and in what order. It is a
working plan, not a contract: dates are deliberately omitted, and sequencing can
change as we learn. See [README.md](README.md) for what the project is.

## Vision

A fast, dependable, cross-platform tool that makes a photogrammetry asset
library feel local: search it, fetch exactly the quality and formats you need,
and get finished materials into your DCC in one click — without a heavyweight
launcher and without lock-in.

## Guiding principles

1. **Engine first, UI second.** Core and service crates stay GUI-agnostic so the
   same engine drives the desktop app and the CLI.
2. **No lock-in.** Assets land in a standardized, documented folder layout; the
   local library index is portable and readable on its own.
3. **Offline-friendly.** Anything you have cached or downloaded remains browsable
   and re-exportable without a network connection.
4. **Respect the platform.** Use Fab's official APIs with the user's own account,
   honor the ToS, and never redistribute asset content.
5. **Small and quick.** Native, low-memory, fast startup — a deliberate contrast
   to Electron-class tools.

## Milestones

Each milestone has a version target and a **definition of done (DoD)**. A
milestone is complete only when its DoD is fully checked.

### M0 — Foundations → `v0.1.0`

Stand up the workspace so contributors can build, test, and trust the toolchain.

- [ ] Cargo workspace with all crates from the README architecture section
      scaffolded (compiling stubs are fine).
- [ ] `rust-toolchain.toml` pinning a stable toolchain.
- [ ] CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on
      Windows, macOS, and Linux.
- [ ] `assetcraft-core`: domain model skeleton, shared `Error` type, config
      loading, and structured logging (`tracing`).
- [ ] Dual license files present (`LICENSE-APACHE`, `LICENSE-MIT`).
- [ ] `docs/architecture.md` describing the crate graph and data flow.

**DoD:** `cargo build --workspace` and `cargo test --workspace` pass on all three
platforms in CI.

### M1 — Catalog & Auth → `v0.2.0`

Read the library. This is the first milestone that talks to Fab.

- [ ] Epic/Fab OAuth device-flow login from the CLI.
- [ ] Tokens stored in the OS keyring; refresh handled transparently.
- [ ] Fab API client with search, pagination, and filters (category, type,
      resolution, tags).
- [ ] Asset metadata model in `assetcraft-core` covering previews, texture maps,
      LODs, and available download options.
- [ ] On-disk response cache with sensible invalidation.

**DoD:** `assetcraft auth login` succeeds and `assetcraft search` lists real
results with metadata, on a clean machine, against the live Fab API.

### M2 — Download Engine → `v0.3.0`

Get bytes on disk, reliably.

- [ ] Concurrent downloads with a bounded worker pool.
- [ ] Resumable transfers; partial state survives restarts.
- [ ] Retry with exponential backoff and clear failure reporting.
- [ ] Quality-tier and format selection per asset.
- [ ] Checksum/integrity verification on completion.
- [ ] Global bandwidth limit and a persistent queue.

**DoD:** A large asset can be downloaded, interrupted mid-transfer, resumed to
completion, and verified — with no corruption.

### M3 — Library → `v0.4.0`

Own what you have, even offline.

- [ ] SQLite-backed local library index.
- [ ] Deduplication by asset ID; re-download and re-export from the library.
- [ ] Thumbnail and preview cache.
- [ ] Offline browse of cached metadata and downloaded assets.
- [ ] Library integrity check / repair command.

**DoD:** With the network disabled, previously downloaded assets are fully
browsable and re-exportable.

### M4 — Blender Export → `v0.5.0`

The first real integration.

- [ ] Standardized PBR folder layout emitted for every asset.
- [ ] Blender exporter that builds a correct PBR material graph from the maps.
- [ ] Python addon bridge for one-click import into the active scene.
- [ ] Per-OS Blender path detection.
- [ ] Export both from the CLI and (stub) programmatic API.

**DoD:** `assetcraft export <asset-id> --dcc blender` produces a scene-ready,
correctly-textured material in Blender.

### M5 — GUI & Packaging → `v1.0.0`

Make it a product.

- [ ] egui desktop app: browse, search, filter, asset detail, download queue,
      library view, settings.
- [ ] Login and account status inside the app.
- [ ] Installers/artifacts for Windows (`.msi`/`.zip`), macOS (`.dmg`), Linux
      (`.AppImage`/`.deb`).
- [ ] Auto-update.
- [ ] Onboarding docs and screenshots in the README.

**DoD:** A non-developer can install AssetCraft, sign in, download an asset, and
export it to Blender without touching a terminal.

## Cross-cutting tracks

Ongoing, not tied to a single milestone:

- **Performance** — startup time, memory ceiling, download throughput.
- **Accessibility & UX** — keyboard navigation, theming, clear error states.
- **Internationalization** — externalize strings ahead of community translations.
- **Testing** — integration tests against recorded/mocked API responses.
- **Docs** — keep README, ROADMAP, and `docs/` current with the code.

## Non-goals

- Redistributing, mirroring, or reselling asset content.
- Bypassing Fab licensing, entitlement, or paywalls.
- A general-purpose 3D editor or renderer.
- Targeting the deprecated legacy Bridge service as a primary backend.

## How to shape this roadmap

- Propose changes via an issue or pull request against this file.
- Tell us which DCC you need next — that directly orders M4's follow-on work.
- Milestone DoD checkboxes are the source of truth for "done"; keep them honest
  when closing work.
