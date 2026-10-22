#!/usr/bin/env python3
"""A local stand-in for the Fab catalog API, for developing the GUI offline.

Fab sits behind bot protection, so the GUI's live path cannot be exercised
against fab.com from a dev box or CI. This stub speaks the same wire shape
(see crates/assetcraft-catalog/src/wire.rs) and generates thumbnails, so the
client, worker, paging, and image pipeline can all be tested end to end.

    pip install pillow
    python3 scripts/fab_stub.py 8791
    ASSETCRAFT_FAB_BASE_URL=http://127.0.0.1:8791 cargo run -p assetcraft-gui
"""

import colorsys
import io
import json
import re
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
