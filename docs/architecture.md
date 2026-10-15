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
| `assetcraft-core` | lib | Domain model, config, error type | M0 |
| `assetcraft-catalog` | lib | Fab API client and response cache | M1 |
| `assetcraft-auth` | lib | Epic OAuth and token storage | M1 |
| `assetcraft-download` | lib | Download queue and transfer engine | M2 |
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

Tests must never reach the network. Fab's JSON API sits behind Cloudflare and
answers automated clients from data-centre addresses with a challenge page, so a
test that depended on it would be flaky by construction — and would misreport a
bot filter as a code failure.
