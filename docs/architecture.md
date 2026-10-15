# Architecture

AssetCraft is a Cargo workspace of small crates. The engine — everything except
the two front ends — is GUI-agnostic, so the CLI and the future desktop app share
one implementation. This document describes the crate graph, the flow of a
couple of representative operations, and the decisions behind the shape.

## Crate graph

```
        assetcraft-cli        assetcraft-gui
        (binary)              (binary, M5)
             │  │                   │
             │  │                   │  (will depend on the same crates)
             │  └──────────┬────────┘
             ▼             ▼
   assetcraft-auth   assetcraft-catalog   assetcraft-download   assetcraft-library
             │             │                       │                    │
             └─────────────┴───────────┬───────────┴────────────────────┘
                                       ▼
                                assetcraft-core
                                       ▲
                                       │
                          assetcraft-dcc-blender
                          (adapter, M4)
```

Dependencies point one way, always toward `assetcraft-core`. Binaries depend on
service and core crates; service crates depend only on core. DCC adapters
implement behaviour defined against core types, so a new integration is additive
rather than a change to the engine.

| Crate | Kind | Responsibility | Milestone |
| --- | --- | --- | --- |
| `assetcraft-core` | lib | Domain model, config, error type, selection | M0 |
| `assetcraft-catalog` | lib | Fab API client and response cache | M1 |
| `assetcraft-auth` | lib | Epic OAuth and token storage | M1 |
| `assetcraft-download` | lib | Download queue, transfer engine, manifest parser | M2 |
| `assetcraft-library` | lib | Local library index | M3 |
| `assetcraft-dcc-blender` | adapter | Blender export | M4 |
| `assetcraft-cli` | bin | Headless front end | M1 |
| `assetcraft-gui` | bin | egui desktop app | M5 |

## Representative flows

### Search (implemented, M1)

```
assetcraft search "mossy rock"
        │
        ▼
Config::load()                         platform dirs + endpoints + user agent
        │
        ▼
FabClient::from_config(&config)        builds a reqwest client
        │
        ▼
FabClient::search(&SearchQuery)        maps the query to Fab parameters
        │
        ▼
GET /i/listings/search?...             (no auth required)
        │
        ▼
wire::SearchResults                    raw, permissive serde types
        │
        ▼
convert::page_from_search              domain translation
        │
        ▼
Page<AssetSummary>                     printed, or emitted as JSON
```

The `wire` → `convert` split is the load-bearing idea: only `wire.rs` knows
Fab's JSON field names, and only `convert.rs` knows how they map to our model.

### Sign-in (implemented, M1)

```
assetcraft login
        │
        ▼
EpicAuth::authorization_url()          https://www.epicgames.com/id/api/redirect?clientId=…
        │
        ▼
   (user signs in in a browser and copies the code)
        │
        ▼
EpicAuth::exchange_code(code)
        │
        ▼
POST /account/api/oauth/token           Basic <launcher client id:secret>
        │
        ▼
EpicToken                               access + refresh, with an absolute expiry
        │
        ▼
FileTokenStore::save()                  owner-only (0600) JSON file
```

### Download (implemented, M2)

```
DownloadFile { url, destination, size?, sha256? }
        │
        ▼
Downloader::download_all(files, on_progress)
        │   at most `concurrency` transfers in flight (a semaphore)
        ▼
per file: attempt loop (up to `max_attempts`)
        │
        ├─ .part exists? ──▶ GET with  Range: bytes=<len>-
        │                       ├─ 206 ──▶ append, hashing the prefix first
        │                       └─ 200 ──▶ server ignored the range; restart
        │
        ├─ 429/5xx/timeout ──▶ sleep base·2ⁿ (capped) and retry
        ├─ other non-2xx    ──▶ fail immediately
        │
        ▼
per chunk: BandwidthLimiter::acquire(len)  (one limiter shared by the batch)
        │
        ▼
verify size and SHA-256 ──▶ rename .part → destination (atomic)
```

Two invariants make this safe: work happens in a `.part` file so the
destination is never half-written, and a `.part` file is always a *prefix* of
the target, which is what makes resuming sound. A checksum mismatch deletes the
part file, because a corrupt file that survives would be resumed from and
reported as complete.

When `DownloadOptions::bandwidth` is set, the byte budget is shared by every
transfer in the batch rather than applied per file, so the cap is on the engine:
ten concurrent transfers still total the limit.

### Asset resolution (implemented, M2)

The step that turns an owned asset into bytes: pick a format and file, ask Fab
for its signed download info, then read the binary manifest that info points at.

```
assetcraft resolve <id> --format fbx --quality 4K
        │
        ▼
FabClient::listing_formats(&id)         GET /i/listings/{id}/asset-formats (no auth)
        │
        ▼
core::select(&formats, &criteria)       Selection { format_code, file.id }
        │
        ▼
FabClient::download_info(id, fmt, file) GET …/files/{file}/download-info (auth)
        │                               ──▶ DownloadInfo { artifact_id, distribution_points }
        ▼
FabClient::fetch_bytes(point.manifest_url)      GET the signed manifest URL
        │
        ▼
Manifest::parse(&bytes)                 chunked manifest (magic 0x44BEC00C)
        │                               ──▶ chunks (to fetch) + files (chunk parts)
        ▼
Manifest::chunk_downloads(base_url)     each chunk's URL under the CDN base
```

`Manifest` is the resolution result: `chunks` are the compressed blobs to
download, and `files` are the logical outputs, each cut from one or more
`chunk_parts` that point into chunks. Reassembling the chunks into files — and
the CDN base URL the chunks hang off — is the remaining piece and is not done
yet. `resolve` stops at printing the plan.

The parser is a faithful port of the layout the wider Epic tooling ecosystem
documents, and its byte layout is pinned by a hand-built fixture in the tests
rather than by a writer we also wrote, so the reader is checked against the
specification. It has **not** been validated against a manifest from a real Fab
account, and encrypted manifests (`stored_as & 0x2`) are rejected rather than
guessed at. See the decision below.

### Format and quality selection (implemented, M2)

```
assetcraft formats <id> --format fbx --quality 4K --prefer largest
        │
        ▼
FabClient::listing_formats(&id)        GET /i/listings/{id}/asset-formats (no auth)
        │
        ▼
convert::formats_from_wire             AssetFormat { format_type, files }
        │
        ▼
core::select(&formats, &criteria)      pure: filter by code, quality text, platform, engine
        │
        ▼
Selection { format_code, format_name, file }
```

The chosen `file.id` is exactly what a download request needs; the step that
turns it into bytes is asset resolution, above.

### Library (implemented, M3)

```
assetcraft library                    ──▶ fetch every page from /e/accounts/{id}/ue/library
                                            │
                                            ▼
                                     Library::insert(item)   INSERT OR IGNORE, so a repeat is a no-op
                                            │
                                            ▼
                                     library.sqlite          JSON item + denormalised title
                                            │
              assetcraft library --offline ──┤
             assetcraft library --search q ──┤── SELECT … ORDER BY title
              assetcraft library --verify  ──┴── read every item back and report what fails
```

Media is cached beside the index, not inside it. `assetcraft library
--thumbnails` fetches each item's thumbnail through the catalog client and
stores it in a content-addressed `MediaCache`, so offline browsing points at a
local file rather than a URL.

## Decisions

### We own the catalog client rather than wrapping `egs-api`

`egs-api` (MIT) already implements Epic auth and much of Fab, and it was used as
a reference for the endpoints. We still wrote our own client so that our domain
types are not dictated by another crate's public API, and so the dependency
surface stays small. That decision can be revisited if the maintenance cost of
tracking Fab outweighs the control.

### TLS comes from `native-tls`, not rustls

`reqwest` 0.13's default TLS is rustls with the `aws-lc-rs` provider, which
requires `cmake` at build time. That is not guaranteed to be present on
developer machines or minimal CI images. `native-tls` uses the platform's TLS
(OpenSSL, Schannel, Secure Transport) and needs no extra build tooling. If we
later want a pure-Rust build everywhere, the alternative is
`rustls-no-provider` plus the `ring` provider installed at startup.

### Tokens sit behind a `TokenStore` trait

The default backend is a private JSON file. An OS keyring backend is preferable
in principle, but a keyring needs a running secret service, which is absent in
CI, containers, and headless Linux — so the file backend is what ships first and
the trait keeps the swap from touching callers.

### Login uses the authorization-code flow, not the device flow

Epic's OAuth device-authorization grant is not available to the public launcher
client: `POST /account/api/oauth/deviceAuthorization` returns `401
authentication_failed` for those credentials, while `client_credentials` on the
same client succeeds (verified 2026-10-16). The roadmap originally called for a
device flow; the browser authorization-code flow is the one that actually works
with a client we can legitimately use.

### The launcher credentials are not secrets

`LAUNCHER_CLIENT_ID` / `LAUNCHER_CLIENT_SECRET` are Epic's public *client*
credentials, embedded in every Epic Games Launcher installation. They identify
the application, not a user, and grant no access on their own — a user still has
to sign in. They are recorded here the same way they appear in other community
tools.

### A response cache keyed by URL, with the URL stored in the entry

`ResponseCache` keys entries by a dependency-free FNV-1a hash of the request URL
and stores the URL inside the entry. On read it verifies the stored URL matches,
so a hash collision degrades to a cache miss rather than serving the wrong body.

### The download engine was built before asset resolution

An asset's files arrive as a binary manifest, and parsing it correctly can only
be confirmed against a manifest produced by a real account. The transport engine
— concurrency, resumption, retries, integrity — has no such dependency: it can
be proven correct here, with tests, against a local server. So the engine was
built and tested first.

The parser then followed from the format the wider Epic tooling ecosystem
documents, rather than being guessed at: its byte layout is pinned by a
hand-built fixture written from that specification, not by a writer we also
wrote, so the reader is checked against the spec instead of against itself. That
is a weaker guarantee than a real manifest gives, and it is stated plainly
rather than hidden: the parser is marked unvalidated against real Fab data, and
a first run against a real manifest is a validation step. Encrypted manifests
are rejected outright. This is the one place the project ships a reader it could
not exercise end to end, and it says so in the module docs, the ROADMAP, and
here.

### Quality is matched by substring, not by a fixed tier list

A Megascans listing carries its quality tier in a file's name or asset type
(`…_4K.zip`, `High`), and the wording differs between sellers, so hard-coding a
tier vocabulary would encode guesses about data we have not seen. Selection
instead matches the requested quality as a case-insensitive substring of those
two fields, and platform and engine version as case-insensitive equality against
the file's lists. `--prefer largest|smallest` breaks ties by reported size, with
an unknown size ranked last in either direction. The result is the `file.id` a
download request needs; it is deliberately not a claim about what the tiers are
called.

### The bandwidth limit is a token bucket that may go into debt

A rate limiter has to hold the *aggregate* rate when several transfers run at
once, and the obvious design — sleep until the bucket has room, then take the
bytes — lets every waiter wake at the same moment and burst past the limit. So a
reservation is taken *before* the wait: the balance is allowed to go negative,
and the caller sleeps for however long that debt takes to clear at the rate. The
next caller sees the debt and waits longer, which serialises the throughput
without a lock held across the sleep. Capping refills at the burst size only
limits the positive side, so an idle period cannot bank throughput it never had.

### The library stores an item as JSON, with one denormalised column

The `assets` table holds the whole `LibraryItem` as JSON, plus a copy of the
title. The JSON is the source of truth, so adding a field to the domain model
cannot silently drop data that is already stored — either the item reads back
exactly, or `verify` reports the row. The title is duplicated only because it is
what we sort and search on, and SQLite can index and order a column but not a
JSON path as cheaply.

Search uses `instr(lower(title), …)` rather than `LIKE`, so a query containing
`%` or `_` matches those characters literally instead of acting as a wildcard.

### Cached media is content-addressed by URL, with the URL kept alongside

The thumbnail cache names each image after a hash of its URL, so re-caching an
item is a no-op and two items that share a preview share one file. A hash alone
could in principle collide, so each image has a sibling `<key>.url` file holding
the source URL; a read whose stored URL does not match the requested one is a
miss rather than the wrong picture. The sidecar is written after the image, so a
half-finished entry is simply refetched. The image itself stays a valid file on
disk, which keeps the cache browsable without the tool.

## Endpoint reference

All Fab paths are relative to `https://www.fab.com`.

| Purpose | Method | Path | Auth |
| --- | --- | --- | --- |
| Search listings | GET | `/i/listings/search` | no |
| Listing detail | GET | `/i/listings/{uid}` | no |
| Asset formats | GET | `/i/listings/{uid}/asset-formats` | no |
| License types | GET | `/i/taxonomy/licenses` | no |
| Tag groups | GET | `/i/tags/groups` | no |
| My library | GET | `/e/accounts/{account_id}/ue/library?count=&cursor=` | yes |
| File download info | GET | `/p/egl/listings/{id}/asset-formats/{fmt}/files/{file}/download-info` | yes |
| Artifact manifest | POST | `/e/artifacts/{artifact_id}/manifest` | yes |

Epic account service: `https://account-public-service-prod03.ol.epicgames.com`,
token endpoint `/account/api/oauth/token`.

## Testing

- **Wire decoding** is tested against responses captured from the live API, so a
  field-name regression fails a test rather than reaching a user.
- **HTTP behaviour** is tested with `httpmock`, exercising real request building
  and response handling without touching the network.
- **Local state** (token permissions, cache round-trips, library de-duplication)
  is tested against temporary directories.
- **The binary manifest parser** is tested against a hand-built fixture whose
  bytes are written from the documented layout and whose section sizes are
  asserted against hand-computed values, so the reader is pinned to the
  specification rather than to a writer we also wrote.

Tests must never reach the network. Fab's JSON API sits behind Cloudflare and
answers automated clients from data-centre addresses with a challenge page, so a
test that depended on it would be flaky by construction — and would misreport a
bot filter as a code failure.
