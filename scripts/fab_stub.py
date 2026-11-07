#!/usr/bin/env python3
"""A local stand-in for the Fab catalog API, for developing the GUI offline.

Fab sits behind bot protection, so the GUI's live path cannot be exercised
against fab.com from a dev box or CI. This stub speaks the same wire shape
(see crates/assetcraft-catalog/src/wire.rs) and generates thumbnails, so the
client, worker, paging, and image pipeline can all be tested end to end.

Downloads are served too: `asset-formats` and `download-info` describe one FBX
file, and `/stub/manifest.bin` is a real binary manifest whose two chunks are
served under `/stub/ChunksV4/`. Both are generated here, so the whole transfer
path — resolve, fetch, reassemble, verify — runs without a network.

    pip install pillow
    python3 scripts/fab_stub.py 8791
    ASSETCRAFT_FAB_BASE_URL=http://127.0.0.1:8791 cargo run -p assetcraft-gui
"""

import colorsys
import hashlib
import io
import json
import re
import struct
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from PIL import Image, ImageDraw

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8791
BASE = f"http://127.0.0.1:{PORT}"

TITLES = [
    ("Weathered Sandstone Wall", "Dune Works", "material", False, 4.7),
    ("Mossy Granite Boulder", "Rock Forge", "3d-model", True, 4.9),
    ("Coastal Cliff Face", "Rock Forge", "3d-model", False, 4.8),
    ("Palm Grove Cluster", "Greenhouse", "3d-plant", True, 4.6),
    ("Cracked Asphalt Tiles", "Urban Decay", "material", False, 4.4),
    ("Rust Streak Decals", "Urban Decay", "decal", True, 4.2),
    ("Desert Dune Sand", "Dune Works", "material", True, 4.5),
    ("Oak Chest Prop", "Prop Shop", "3d-model", True, 4.9),
    ("Fern Understory", "Greenhouse", "3d-plant", False, 4.7),
    ("Graffiti Wall Decals", "Street Layer", "decal", True, 4.1),
    ("Gravel Ground Scan", "Dune Works", "material", False, 4.3),
    ("Basalt Column Set", "Rock Forge", "3d-model", False, 4.6),
    ("Ivy Wall Cover", "Greenhouse", "3d-plant", False, 4.8),
    ("Peeling Paint Pack", "Urban Decay", "decal", True, 4.0),
    ("Coral Rubble Scan", "Reef Lab", "3d-model", True, 4.4),
    ("Seagrass Patch", "Reef Lab", "3d-plant", True, 4.5),
    ("Mud Splatter Decals", "Street Layer", "decal", False, 4.2),
    ("Basalt Cobblestone", "Rock Forge", "material", False, 4.8),
    ("Pine Sapling", "Greenhouse", "3d-plant", True, 4.6),
    ("Barrel Crate Stack", "Prop Shop", "3d-model", False, 4.3),
    ("Tire Track Decals", "Street Layer", "decal", True, 3.9),
    ("City Skyline Kit", "Block Works", "3d-scene", False, 4.7),
    ("Fishing Village Kit", "Block Works", "3d-scene", True, 4.5),
    ("Concrete Barrier Set", "Prop Shop", "3d-model", False, 4.1),
]

TYPE_LABELS = {
    "3d-model": "3D Model",
    "3d-scene": "3D Scene",
    "3d-plant": "3D Plant",
    "material": "Material",
    "decal": "Decal",
}

LISTINGS = [
    {
        "uid": f"stub-{n:03}",
        "title": title,
        "seller": seller,
        "listingType": listing_type,
        "isFree": free,
        "rating": rating,
        "n": n,
    }
    for n, (title, seller, listing_type, free, rating) in enumerate(TITLES)
]

THUMBS = {}


def thumbnail(n):
    if n in THUMBS:
        return THUMBS[n]
    width, height = 480, 270
    image = Image.new("RGB", (width, height), (22, 23, 28))
    draw = ImageDraw.Draw(image)
    for y in range(height):
        draw.line(
            [(0, y), (width, y)],
            fill=(18 + y // 30, 20 + y // 28, 26 + y // 24),
        )
    hue = ((n * 47) % 360) / 360.0
    sphere = tuple(int(c * 255) for c in colorsys.hsv_to_rgb(hue, 0.55, 0.82))
    highlight = tuple(int(c * 255) for c in colorsys.hsv_to_rgb(hue, 0.22, 1.0))
    cx, cy, r = width // 2, height // 2 + 6, 74
    draw.ellipse([cx - r * 1.05, cy + r * 0.78, cx + r * 1.05, cy + r * 1.12], fill=(9, 10, 12))
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=sphere)
    draw.ellipse(
        [cx - r * 0.62, cy - r * 0.74, cx - r * 0.02, cy - r * 0.10],
        fill=highlight,
    )
    # A suggestion of a scanned surface under the subject.
    draw.rectangle([0, height - 30, width, height], fill=(14, 15, 19))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    THUMBS[n] = buffer.getvalue()
    return THUMBS[n]


def summary(listing):
    n = listing["n"]
    return {
        "uid": listing["uid"],
        "title": listing["title"],
        "listingType": listing["listingType"],
        "user": {"sellerName": listing["seller"]},
        "category": {"name": TYPE_LABELS.get(listing["listingType"], "Asset")},
        "isMature": False,
        "isFree": listing["isFree"],
        "isDiscounted": False,
        "publishedAt": f"2026-0{1 + n % 9}-1{n % 9}T12:00:00.000Z",
        "tags": [
            {"name": listing["title"].split()[0].lower()},
            {"name": listing["listingType"].replace("-", " ")},
        ],
        "thumbnails": [
            {"mediaUrl": f"{BASE}/img/{n}.png", "type": "thumbnail"},
        ],
        "ratings": {"averageRating": listing["rating"], "total": 40 + n},
    }


#: Assets the stub account "owns", with paging across two pages.
OWNED = [LISTINGS[n] for n in (0, 3, 7, 12, 15)]


def owned_item(listing):
    n = listing["n"]
    return {
        "assetId": listing["uid"],
        "assetNamespace": "fab",
        "title": listing["title"],
        "description": f"{listing['title']} - owned in the stub account.",
        "url": f"https://www.fab.com/listings/{listing['uid']}",
        "source": "fab",
        "distributionMethod": "download",
        "categories": [{"name": TYPE_LABELS.get(listing["listingType"], "Asset")}],
        "images": [{"url": f"{BASE}/img/{n}.png", "type": "thumbnail"}],
    }


def detail(listing):
    body = summary(listing)
    body.update(
        {
            "description": (
                f"<p><b>{listing['title']}</b> is a scanned, game-ready asset from "
                f"{listing['seller']}.</p><p>Includes 4 LODs, PBR maps at 4K, and a "
                f"collision mesh. Suitable for real-time and offline rendering.</p>"
            ),
            "createdAt": "2026-02-14T09:30:00.000Z",
            "reviewCount": 40 + listing["n"],
        }
    )
    return body


#: The code the stand-in sign-in page hands out.
STUB_CODE = "stub-code"


def oauth_token(display_name="Stub Player", account_id="acc-stub"):
    return {
        "access_token": "stub-access-token",
        "refresh_token": "stub-refresh-token",
        "token_type": "bearer",
        "expires_in": 7200,
        "account_id": account_id,
        "displayName": display_name,
    }


#: What every stub download produces, and the chunks its files arrive in.
DOWNLOAD_FILES = [
    ("Content/stub_mesh.uasset", b"AssetCraft stub mesh payload\n" * 40),
    ("Content/stub_texture.uasset", b"AssetCraft stub texture payload\n" * 90),
]

MANIFEST_MAGIC = 0x44BEC00C
CHUNK_MAGIC = 0xB1FE3AA2
CHUNK_HEADER_SIZE = 66


def _u32(value):
    return struct.pack("<I", value)


def _u64(value):
    return struct.pack("<Q", value)


def _i64(value):
    return struct.pack("<q", value)


def _guid(words):
    return b"".join(_u32(word) for word in words)


def _fstring(text):
    if not text:
        return _u32(0)
    raw = text.encode()
    return _u32(len(raw) + 1) + raw + b"\0"


def _section(build):
    body = bytearray()
    build(body)
    return _u32(len(body) + 4) + bytes(body)


def stub_chunks():
    chunks = []
    for n, (name, data) in enumerate(DOWNLOAD_FILES):
        chunks.append(
            {
                "name": name,
                "guid": [n + 1, n + 2, n + 3, n + 4],
                "hash": 0x1122334455667788 + n,
                "sha1": hashlib.sha1(data).digest(),
                "group": n % 100,
                "data": data,
            }
        )
    return chunks


def chunk_relative_path(chunk):
    guid = "".join(f"{word:08X}" for word in chunk["guid"])
    return f"ChunksV4/{chunk['group']:02}/{chunk['hash']:016X}_{guid}.chunk"


def chunk_file(chunk):
    payload = chunk["data"]
    header = (
        _u32(CHUNK_MAGIC)
        + _u32(3)
        + _u32(CHUNK_HEADER_SIZE)
        + _u32(len(payload))
        + _guid(chunk["guid"])
        + _u64(chunk["hash"])
        + bytes([0])
        + chunk["sha1"]
        + bytes([0])
        + _u32(len(payload))
    )
    return header + payload


def manifest_bytes():
    """A version 18, uncompressed manifest for the stub's two files."""
    chunks = stub_chunks()

    def meta(out):
        out.extend(bytes([0]))
        out.extend(_u32(18))  # feature level
        out.extend(bytes([0]))  # is_file_data
        out.extend(_u32(0))  # app id
        out.extend(_fstring("AssetCraft Stub"))
        out.extend(_fstring("1.0"))
        out.extend(_fstring("stub.exe"))
        out.extend(_fstring(""))
        out.extend(_u32(0))  # prerequisites
        out.extend(_fstring(""))
        out.extend(_fstring(""))
        out.extend(_fstring(""))

    def chunk_list(out):
        out.extend(bytes([0]))
        out.extend(_u32(len(chunks)))
        for chunk in chunks:
            out.extend(_guid(chunk["guid"]))
        for chunk in chunks:
            out.extend(_u64(chunk["hash"]))
        for chunk in chunks:
            out.extend(chunk["sha1"])
        for chunk in chunks:
            out.append(chunk["group"])
        for _ in chunks:
            out.extend(_u32(1024 * 1024))
        for chunk in chunks:
            out.extend(_i64(len(chunk_file(chunk))))

    def file_list(out):
        out.extend(bytes([0]))
        out.extend(_u32(len(chunks)))
        for chunk in chunks:
            out.extend(_fstring(chunk["name"]))
        for _ in chunks:
            out.extend(_fstring(""))  # symlink target
        for chunk in chunks:
            out.extend(hashlib.sha1(chunk["data"]).digest())
        for _ in chunks:
            out.append(0)  # flags
        for _ in chunks:
            out.extend(_u32(0))  # install tags
        for chunk in chunks:
            out.extend(_u32(1))  # chunk parts
            out.extend(_u32(28))  # part record size
            out.extend(_guid(chunk["guid"]))
            out.extend(_u32(0))  # offset into the chunk
            out.extend(_u32(len(chunk["data"])))

    def custom_fields(out):
        out.extend(bytes([0]))
        out.extend(_u32(0))

    body = (
        _section(meta)
        + _section(chunk_list)
        + _section(file_list)
        + _section(custom_fields)
    )
    header = (
        _u32(MANIFEST_MAGIC)
        + _u32(41)  # header size
        + _u32(len(body))
        + _u32(len(body))
        + bytes(20)  # sha, unchecked when uncompressed
        + bytes([0])  # stored_as: uncompressed
        + _u32(18)  # manifest version
    )
    return header + body


MANIFEST = manifest_bytes()
CHUNK_FILES = {
    chunk_relative_path(chunk): chunk_file(chunk) for chunk in stub_chunks()
}
DOWNLOAD_SIZE = sum(len(data) for _, data in DOWNLOAD_FILES)


def format_body():
    return {
        "assetFormatType": {"code": "fbx", "name": "FBX"},
        "files": [
            {
                "uid": "stub-file",
                "name": "Stub Asset (4K)",
                "fileSize": DOWNLOAD_SIZE,
                "assetType": "3D Model",
                "platformsIncluded": ["Windows"],
                "engineVersions": ["5.4"],
            }
        ],
    }


def redirect_page():
    body = f"""<!doctype html>
<html><head><title>Sign in</title></head>
<body style="font-family: sans-serif; background: #1b1b1b; color: #ddd; padding: 40px">
<h1>AssetCraft developer stub</h1>
<p>This stands in for Epic's sign-in redirect. The authorization code is:</p>
<pre style="font-size: 20px; background: #2a2a2a; padding: 12px">{STUB_CODE}</pre>
<p>Paste it into the AssetCraft sign-in dialog.</p>
</body></html>"""
    return body.encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):  # noqa: N802 - http.server API
        url = urlparse(self.path)
        if url.path == "/i/listings/search":
            query = parse_qs(url.query)
            cursor = query.get("cursor", [None])[0]
            if cursor:
                items = LISTINGS[12:]
                next_cursor = None
            else:
                items = LISTINGS[:12]
                next_cursor = "page-2"
            body = {
                "results": [summary(item) for item in items],
                "count": len(LISTINGS),
                "cursors": {"next": next_cursor, "previous": None},
            }
            self.reply_json(body)
            return

        # The authenticated endpoints start with a CSRF handshake.
        if url.path == "/i/csrf":
            self.reply_json({"ok": True})
            return

        match = re.fullmatch(r"/i/listings/[\w-]+/asset-formats", url.path)
        if match:
            self.reply_json([format_body()])
            return

        match = re.fullmatch(
            r"/p/egl/listings/[\w-]+/asset-formats/[\w-]+/files/[\w-]+/download-info",
            url.path,
        )
        if match:
            self.reply_json(
                {
                    "artifactId": "stub-artifact",
                    "assetFormat": "fbx",
                    "distributionPoints": [
                        {"manifestUrl": f"{BASE}/stub/manifest.bin"}
                    ],
                }
            )
            return

        if url.path == "/stub/manifest.bin":
            self.reply_bytes(MANIFEST, "application/octet-stream")
            return

        if url.path.startswith("/stub/ChunksV4/"):
            chunk = CHUNK_FILES.get(url.path[len("/stub/"):])
            if chunk is None:
                self.send_error(404)
                return
            self.reply_bytes(chunk, "application/octet-stream")
            return

        match = re.fullmatch(r"/e/accounts/[\w-]+/ue/library", url.path)
        if match:
            query = parse_qs(url.query)
            cursor = query.get("cursor", [None])[0]
            if cursor:
                items = OWNED[3:]
                next_cursor = None
            else:
                items = OWNED[:3]
                next_cursor = "lib-2"
            body = {
                "results": [owned_item(item) for item in items],
                "cursors": {"next": next_cursor},
            }
            self.reply_json(body)
            return

        match = re.fullmatch(r"/i/listings/[\w-]+", url.path)
        if match:
            uid = url.path.rsplit("/", 1)[1]
            listing = next((item for item in LISTINGS if item["uid"] == uid), None)
            if listing is None:
                self.send_error(404)
                return
            self.reply_json(detail(listing))
            return

        if url.path == "/id/api/redirect":
            self.reply_bytes(redirect_page(), "text/html; charset=utf-8")
            return

        match = re.fullmatch(r"/img/(\d+)\.png", url.path)
        if match:
            self.reply_bytes(thumbnail(int(match.group(1))), "image/png")
            return

        self.send_error(404)

    def do_POST(self):  # noqa: N802 - http.server API
        url = urlparse(self.path)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode()
        form = parse_qs(body)

        if url.path == "/account/api/oauth/token":
            grant = form.get("grant_type", [""])[0]
            code = form.get("code", [""])[0]
            refresh = form.get("refresh_token", [""])[0]
            if grant == "authorization_code" and code == STUB_CODE:
                self.reply_json(oauth_token())
            elif grant == "refresh_token" and refresh == "stub-refresh-token":
                self.reply_json(oauth_token())
            else:
                self.reply_json(
                    {
                        "errorCode": "invalid_grant",
                        "errorMessage": "the code or refresh token is not the stub's",
                    },
                    status=400,
                )
            return

        self.send_error(404)

    def reply_json(self, payload, status=200):
        self.reply_bytes(json.dumps(payload).encode(), "application/json", status)

    def reply_bytes(self, payload, content_type, status=200):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt, *args):
        print(f"{self.command} {self.path} - {fmt % args}", flush=True)


if __name__ == "__main__":
    print(f"fab stub listening on {BASE}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
