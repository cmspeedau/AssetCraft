# AssetCraft — Roadmap

This document tracks where AssetCraft is going and in what order. It is a
working plan, not a contract: dates are deliberately omitted, and sequencing can
change as we learn. See [README.md](README.md) for what the project is.

## Current status

- **M0 — Foundations:** complete.
- **M1 — Catalog & Auth:** in progress. Public catalog reads, sign-in, token
  storage and refresh, the response cache, and the authenticated library and
  download-info endpoints are in. The OS keyring backend remains.
- **M2 — Download Engine:** in progress. The transport engine (bounded
  concurrency, resumption, retries, integrity verification, a global bandwidth
  limit) and the persistent queue are in; resolving an asset into files needs
  the binary manifest parser.
- **M3 — Library:** in progress. A SQLite library backs offline browse, search,
  an integrity check, and a thumbnail cache; re-export from the library
  remains.

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

- [x] Cargo workspace with all crates from the README architecture section
      scaffolded (compiling stubs are fine).
- [x] `rust-toolchain.toml` pinning a stable toolchain.
- [x] CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on
      Windows, macOS, and Linux.
- [x] `assetcraft-core`: domain model skeleton, shared `Error` type, config
      loading, and structured logging (`tracing`).
- [x] Dual license files present (`LICENSE-APACHE`, `LICENSE-MIT`).
- [x] `docs/architecture.md` describing the crate graph and data flow.

**DoD:** `cargo build --workspace` and `cargo test --workspace` pass on all three
platforms in CI.

### M1 — Catalog & Auth → `v0.2.0`

Read the library. This is the first milestone that talks to Fab.

- [x] Fab API client with search, pagination, and filters (type, category, sort,
      free/discounted, seller).
- [x] Sign-in with Epic, storing the token behind a `TokenStore` (a private file
      today; see `docs/architecture.md` for why the OS keyring is deferred).
- [x] Transparent refresh: commands resolve a usable token before calling an
      authenticated endpoint.
- [x] On-disk response cache for public reads, with a time-to-live.
- [x] Asset metadata model covering previews, categories, formats, and download
      locations.
- [x] Authenticated library and download-info endpoints, including the Fab CSRF
      handshake they require.
- [ ] OS keyring token backend.
- [ ] Texture-map and LOD metadata (lands with the download engine, M2).

> **Note on the device flow.** The original plan called for an OAuth *device
> flow*. Epic's device-authorization endpoint answers `401` for the public
> launcher client while `client_credentials` on the same client succeeds, so the
> browser authorization-code flow is what ships.

**DoD:** `assetcraft login` succeeds on a clean machine and `assetcraft search`
lists real results with metadata against the live Fab API.

> **Verification note.** The live Fab JSON API sits behind Cloudflare and answers
> data-centre clients with a challenge, so the live end-to-end check is a manual
> step rather than a CI one. Everything up to the network boundary is covered by
> mocked tests.

### M2 — Download Engine → `v0.3.0`

Get bytes on disk, reliably.

- [x] Concurrent downloads with a bounded worker pool.
- [x] Resumable transfers: a partial file is a prefix, resumed with a range
      request, and survives a restart.
- [x] Retry with exponential backoff, capped, and clear failure reporting.
      Client errors are not retried.
- [x] Checksum and size verification on completion, in a `.part` file that is
      renamed into place only once it verifies.
- [x] A persistent, de-duplicating download queue.
- [ ] Resolve an owned asset into the files to fetch: parse Fab's signed binary
      artifact manifest (Epic's chunked manifest format, magic `0x44BEC00C`).
- [ ] Quality-tier and format selection per asset.
- [x] Global bandwidth limit.

> **Ordering note.** The transport engine and the manifest parser were split.
> The engine can be proven correct with tests against a local server; the
> manifest parser can only be validated against a manifest from a real account,
> so it follows once that is available. See `docs/architecture.md`.

**DoD:** A large asset can be downloaded, interrupted mid-transfer, resumed to
completion, and verified — with no corruption.

### M3 — Library → `v0.4.0`

Own what you have, even offline.

- [x] SQLite-backed local library index.
- [x] De-duplication by asset id: syncing the same asset twice is a no-op.
- [x] Offline browse of cached metadata (`assetcraft library --offline`,
      `assetcraft library --search <text>`).
- [x] Library integrity check (`assetcraft library --verify`).
- [x] Thumbnail and preview cache (`assetcraft library --thumbnails`).
- [ ] Re-download and re-export straight from the library (needs M2 asset
      resolution and the M4 exporter).
- [ ] Repair damaged entries, rather than only reporting them.

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
