#!/usr/bin/env python3
"""Generate every Conduit brand asset from assets/brand/conduit.mark.json.

This is the ONLY place in the repository that turns the mark into pixels or
into code. The previous state of the repo had four independent definitions of
the logo (logo.svg, ConduitLogo.tsx, conduit_logo.dart and generate_icons.py)
that had drifted apart and produced ~29 image files with ~29 distinct hashes.

Design rules that fall out of having one generator:

  * One rasteriser, pinned (Pillow), rendering at SS x target then box-filters
    down. Supersampling is why these icons have real antialiasing; the old
    script drew straight to the final size and only ever emitted alpha {0,255}.
  * Same size + same variant => byte-identical file, every time, so
    "the same logo everywhere" is an enforceable invariant rather than a hope.
  * Rounded-rect plate, gradient plate and the bare mark are declared variants
    in the spec, not separate hand-drawn artwork.
  * icon.ico is written from the LARGEST frame. Pillow silently drops any frame
    larger than the base image, which is why the committed icon.ico contained a
    single 16x16 frame while the generator reported six.

Requires: Pillow  (pip install Pillow)

Usage:
    python scripts/icons/build_icons.py            # write assets + manifest
    python scripts/icons/build_icons.py --check    # verify only, write nothing
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile

try:
    from PIL import Image, ImageDraw
except ImportError:  # pragma: no cover
    sys.exit("Pillow is required: pip install Pillow")

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SPEC_PATH = os.path.join(REPO, "assets", "brand", "conduit.mark.json")
MANIFEST_PATH = os.path.join(REPO, "assets", "brand", "icon-manifest.json")


# --------------------------------------------------------------------------
# spec
# --------------------------------------------------------------------------
def load_spec() -> dict:
    with open(SPEC_PATH, "r", encoding="utf-8") as fh:
        return json.load(fh)


def hex_rgb(value: str) -> tuple[int, int, int]:
    value = value.lstrip("#")
    if len(value) == 8:  # RRGGBBAA
        value = value[:6]
    return tuple(int(value[i:i + 2], 16) for i in (0, 2, 4))  # type: ignore[return-value]


# --------------------------------------------------------------------------
# geometry
# --------------------------------------------------------------------------
class Mark:
    """The conduit: a constant-thickness tube swept along a polyline.

    The silhouette is the union of one rotated rectangle per segment plus a
    disc at each interior joint. The two terminals get no disc, so they are
    flat sections cut perpendicular to the axis - that is what makes the mark
    read as a sectioned pipe rather than as a letterform.
    """

    def __init__(self, spec: dict):
        mark = spec["mark"]
        self.grid = float(spec["grid"])
        self.width = float(mark["tubeWidth"])
        self.joint_radius = float(mark["jointRadius"])
        self.terminal_cap = str(mark["terminalCap"])
        self.centerline = [(float(p["x"]), float(p["y"])) for p in mark["centerline"]]
        if self.terminal_cap != "flat":
            raise SystemExit(f'unsupported terminalCap: {self.terminal_cap!r}')
        if abs(self.joint_radius - self.width / 2.0) > 1e-9:
            raise SystemExit("jointRadius must equal tubeWidth / 2")
        if len(self.centerline) < 3:
            raise SystemExit("the conduit needs at least three centreline points")

    def segments(self) -> list[tuple[tuple[float, float], tuple[float, float], float]]:
        out = []
        for a, b in zip(self.centerline, self.centerline[1:]):
            out.append((a, b, self.width))
        return out

    def joints(self) -> list[tuple[float, float, float]]:
        return [(x, y, self.joint_radius) for x, y in self.centerline[1:-1]]

    def bbox(self) -> tuple[float, float, float, float]:
        """Axis-aligned bounds of the tube, in grid units."""
        h = self.width / 2.0
        xs: list[float] = []
        ys: list[float] = []
        for (x0, y0), (x1, y1), w in self.segments():
            dx, dy = x1 - x0, y1 - y0
            ln = math.hypot(dx, dy)
            nx, ny = -dy / ln * h, dx / ln * h
            for px, py in ((x0 + nx, y0 + ny), (x0 - nx, y0 - ny),
                           (x1 + nx, y1 + ny), (x1 - nx, y1 - ny)):
                xs.append(px)
                ys.append(py)
        for cx, cy, r in self.joints():
            xs.extend((cx - r, cx + r))
            ys.extend((cy - r, cy + r))
        return min(xs), min(ys), max(xs), max(ys)

    def center(self) -> tuple[float, float]:
        x0, y0, x1, y1 = self.bbox()
        return (x0 + x1) / 2.0, (y0 + y1) / 2.0

    # -- rasterisation ----------------------------------------------------
    def draw(self, draw: "ImageDraw.ImageDraw", k: float, color: tuple[int, int, int],
             ox: float = 0.0, oy: float = 0.0) -> None:
        """Paint the mark.

        ``k`` is grid-units -> pixels. ``ox``/``oy`` shift the mark in pixels,
        which the tray variant uses to sit the tube optically centred in the
        menu-bar slot rather than merely on the grid centre.
        """
        h = self.width * k / 2.0
        for (x0, y0), (x1, y1), _w in self.segments():
            dx, dy = x1 - x0, y1 - y0
            ln = math.hypot(dx, dy)
            nx, ny = -dy / ln * h, dx / ln * h
            ax, ay = x0 * k + ox, y0 * k + oy
            bx, by = x1 * k + ox, y1 * k + oy
            draw.polygon([(ax + nx, ay + ny), (bx + nx, by + ny),
                          (bx - nx, by - ny), (ax - nx, ay - ny)], fill=color)
        for cx, cy, r in self.joints():
            ccx, ccy, rr = cx * k + ox, cy * k + oy, r * k
            draw.ellipse([ccx - rr, ccy - rr, ccx + rr, ccy + rr], fill=color)


# --------------------------------------------------------------------------
# variants
# --------------------------------------------------------------------------
def render(spec: dict, mark: Mark, variant_name: str, size: int) -> Image.Image:
    variant = spec["variants"][variant_name]
    ss = int(spec["rendering"]["supersample"])
    resample = getattr(Image, spec["rendering"]["resample"])
    pal = spec["palette"]
    big = size * ss

    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    if variant["plate"]:
        top, bot = hex_rgb(pal["plateTop"]), hex_rgb(pal["plateBottom"])
        for y in range(big):
            t = y / max(big - 1, 1)
            draw.line([(0, y), (big, y)],
                      fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bot)))

    mark_scale = float(variant.get("markScale", 1.0))
    k = (big / mark.grid) * mark_scale
    # Re-centre the tube on its own visual centre so the tray variant sits
    # correctly in the menu bar regardless of where the geometry sits.
    cx, cy = mark.center()
    ox = (big / 2.0) - (cx * k)
    oy = (big / 2.0) - (cy * k)
    mark.draw(draw, k, hex_rgb(pal[variant["markColor"]]), ox, oy)

    if variant["plate"] and not variant["opaque"]:
        radius = variant["cornerRadius"] * big
        mask = Image.new("L", (big, big), 0)
        ImageDraw.Draw(mask).rounded_rectangle(
            [0, 0, big - 1, big - 1], radius=radius, fill=255)
        img.putalpha(mask)
    elif variant["opaque"]:
        img.putalpha(255)

    return img.resize((size, size), resample)


# --------------------------------------------------------------------------
# containers
# --------------------------------------------------------------------------
ICNS_TYPES = {16: b"icp4", 32: b"icp5", 64: b"icp6", 128: b"ic07",
              256: b"ic08", 512: b"ic09", 1024: b"ic10"}


def encode_ico(images: dict[int, Image.Image]) -> bytes:
    """Multi-size .ico. The base frame is the LARGEST image, because Pillow
    silently omits any frame bigger than the image it was handed."""
    sizes = sorted(images)
    base = images[sizes[-1]]
    buf = io.BytesIO()
    base.save(buf, format="ICO", sizes=[(s, s) for s in sizes],
              append_images=[images[s] for s in sizes[:-1]])
    return buf.getvalue()


def encode_icns(images: dict[int, Image.Image]) -> bytes:
    chunks = []
    for size in sorted(images):
        if size not in ICNS_TYPES:
            raise SystemExit(f"no ICNS type for {size}px")
        buf = io.BytesIO()
        images[size].save(buf, format="PNG")
        payload = buf.getvalue()
        chunks.append(ICNS_TYPES[size] + struct.pack(">I", len(payload) + 8) + payload)
    body = b"".join(chunks)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


# --------------------------------------------------------------------------
# driver
# --------------------------------------------------------------------------
def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _dart_format(source: str) -> str:
    """Run `dart format` over a generated Dart file's contents.

    Returns the source unchanged if Dart is unavailable, so the generator still
    works on a machine with only Python. CI has Dart and enforces the format
    gate, and `icon drift` compares hashes, so a stale local format is caught
    rather than shipped.
    """
    exe = shutil.which("dart") or shutil.which("flutter")
    if exe is None:
        return source
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "out.dart")
        with open(path, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(source)
        try:
            subprocess.run([exe, "format", "--output=write", path],
                           check=True, capture_output=True, timeout=120)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
            return source
        with open(path, "r", encoding="utf-8", newline="") as fh:
            return fh.read()


def build() -> dict[str, str]:
    spec = load_spec()
    mark = Mark(spec)
    produced: dict[str, str] = {}

    def emit_bytes(rel: str, data: bytes) -> None:
        path = os.path.join(REPO, rel.replace("/", os.sep))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as fh:
            fh.write(data)
        produced[rel.replace(os.sep, "/")] = sha256(data)

    def emit_png(rel: str, size: int, variant: str) -> None:
        buf = io.BytesIO()
        render(spec, mark, variant, size).save(buf, format="PNG")
        emit_bytes(rel, buf.getvalue())

    groups = spec["targets"]

    for entry in groups.get("tauriPng", []):
        emit_png(entry["path"], entry["size"], entry["variant"])
    for entry in groups.get("desktopWeb", []):
        emit_png(entry["path"], entry["size"], entry["variant"])
    for entry in groups.get("tray", []):
        emit_png(entry["path"], entry["size"], entry["variant"])
    for entry in groups.get("mobileAsset", []):
        emit_png(entry["path"], entry["size"], entry["variant"])
    for entry in groups.get("iosAppIcon", []):
        emit_png(entry["path"], entry["size"], entry["variant"])
    for entry in groups.get("androidMipmap", []):
        emit_png(entry["path"], entry["size"], entry["variant"])

    for entry in groups.get("tauriIco", []):
        images = {s: render(spec, mark, entry["variant"], s) for s in entry["sizes"]}
        emit_bytes(entry["path"], encode_ico(images))
    for entry in groups.get("tauriIcns", []):
        images = {s: render(spec, mark, entry["variant"], s) for s in entry["sizes"]}
        emit_bytes(entry["path"], encode_icns(images))

    from emit_sources import dart_for, svg_for, tsx_for  # noqa: E402
    accent = spec["palette"]["accent"]
    dart_path = spec["emittedSources"]["dart"]["path"]

    # `dart format` is the CI gate for the Flutter app. Hand-matching its exact
    # output from a template is whitespace-fragile, so normalise with the real
    # formatter when it is on PATH. Deterministic for a given Dart version, and
    # the manifest hash still catches any drift.
    dart_src = dart_for(mark, accent)
    dart_src = _dart_format(dart_src)

    for key, text in (("svg", svg_for(spec, mark, int(spec["emittedSources"]["svg"]["grid"]))),
                      ("tsx", tsx_for(mark, accent)),
                      ("dart", dart_src)):
        emit_bytes(spec["emittedSources"][key]["path"], text.encode("utf-8"))

    return produced


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true",
                        help="fail if any tracked asset differs from a fresh render")
    args = parser.parse_args()

    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

    spec = load_spec()
    mark = Mark(spec)
    produced = build()

    if args.check:
        from verify_icons import run  # noqa: E402
        return run(produced, spec)

    manifest = {
        "$comment": "Generated by scripts/icons/build_icons.py. Do not hand-edit.",
        "rasterizer": spec["rendering"]["rasterizer"],
        "supersample": spec["rendering"]["supersample"],
        "resample": spec["rendering"]["resample"],
        "spec_sha256": sha256(open(SPEC_PATH, "rb").read()),
        "count": len(produced),
        "files": dict(sorted(produced.items())),
    }
    with open(MANIFEST_PATH, "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=False)
        fh.write("\n")

    print(f"Conduit icons: {len(produced)} files written.")
    print(f"  mark         {mark.width:g}u tube, centreline {mark.centerline}")
    print(f"  bounds       {tuple(round(v, 2) for v in mark.bbox())} of {mark.grid:g}u")
    print(f"  spec         assets/brand/conduit.mark.json (v{spec['version']})")
    print("  manifest     assets/brand/icon-manifest.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
