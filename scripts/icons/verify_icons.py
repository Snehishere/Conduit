#!/usr/bin/env python3
"""Prove every Conduit brand asset in the repo came from conduit.mark.json.

`build_icons.py --check` regenerates into memory and calls run() here. Checks,
in order of how much damage each one has caught historically:

  A. no duplicates      same (size, variant) must be ONE canonical file
  B. identity           every tracked file matches a fresh render byte for byte
  C. closed set         no tracked file on disk is missing from the spec, and
                        no spec entry is missing from disk
  D. containers         icon.ico has every declared frame, and each frame's
                        payload is byte-equal to the standalone PNG of that size;
                        icon.icns walks cleanly to EOF with every declared type
  E. alpha contract     the bare/plate variants really are antialiased, and the
                        opaque variants really are opaque square
  F. cross-representation  the constants baked into the generated SVG, TSX and
                        Dart match the spec, so the in-app vector mark cannot
                        quietly drift from the baked rasters
  G. platform wiring    tauri.conf.json / index.html / AndroidManifest reference
                        only files the spec actually produces

Exit code 0 = in sync. Non-zero = print what drifted and how to fix it.
"""
from __future__ import annotations

import io
import json
import math
import os
import re
import struct
import sys

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SPEC_PATH = os.path.join(REPO, "assets", "brand", "conduit.mark.json")

ICNS_TYPES = {16: b"icp4", 32: b"icp5", 64: b"icp6", 128: b"ic07",
              256: b"ic08", 512: b"ic09", 1024: b"ic10"}


class Report:
    def __init__(self) -> None:
        self.problems: list[str] = []
        self.checks = 0

    def ok(self) -> None:
        self.checks += 1

    def fail(self, message: str) -> None:
        self.checks += 1
        self.problems.append(message)


def _abs(rel: str) -> str:
    return os.path.join(REPO, rel.replace("/", os.sep))


def _read(rel: str) -> bytes:
    with open(_abs(rel), "rb") as fh:
        return fh.read()


def _png_size(data: bytes) -> tuple[int, int]:
    return struct.unpack(">II", data[16:24])


def _png_alpha_levels(data: bytes) -> set[int]:
    from PIL import Image
    img = Image.open(io.BytesIO(data)).convert("RGBA")
    return set(img.getchannel("A").getdata())


def _png_corner_alpha(data: bytes) -> int:
    from PIL import Image
    return Image.open(io.BytesIO(data)).convert("RGBA").getpixel((0, 0))[3]


def _partial_alpha(data: bytes) -> bool:
    from PIL import Image
    alpha = Image.open(io.BytesIO(data)).convert("RGBA").getchannel("A")
    return any(0 < a < 255 for a in alpha.getdata())


def _entry_for(declared, path: str) -> str | None:
    """Reverse-map a produced path back to its spec variant name."""
    for group in CURRENT_SPEC.get("targets", {}).values():
        if not isinstance(group, list):
            continue
        for entry in group:
            if entry.get("path") == path and "variant" in entry:
                return entry["variant"]
    return None


CURRENT_SPEC: dict = {}


# ---------------------------------------------------------------------------
def run(produced: dict[str, str], spec: dict) -> int:
    from build_icons import Mark, render, encode_ico, encode_icns  # noqa: E402

    global CURRENT_SPEC
    CURRENT_SPEC = spec

    rep = Report()
    mark = Mark(spec)
    groups = spec["targets"]
    variants = spec["variants"]

    # ---------------------------------------------------------------- A
    # Same (size, variant) must be one canonical rendering, byte for byte.
    canonical: dict[tuple, str] = {}
    for group in groups.values():
        if not isinstance(group, list):
            continue
        for entry in group:
            if "size" not in entry:
                continue
            key = (entry["size"], entry["variant"])
            digest = produced.get(entry["path"])
            if digest is None:
                rep.fail(f"A  missing from build output: {entry['path']}")
                continue
            if key in canonical:
                if canonical[key][1] != digest:
                    rep.fail(
                        f"A  {entry['path']} is NOT byte-identical to "
                        f"{canonical[key][0]} although both are "
                        f"{entry['size']}px/{entry['variant']}\n"
                        f"      {canonical[key][0]} = {canonical[key][1][:16]}\n"
                        f"      {entry['path']} = {digest[:16]}")
                    continue
            else:
                canonical[key] = (entry["path"], digest)
            rep.ok()

    # ---------------------------------------------------------------- B
    for rel, digest in sorted(produced.items()):
        path = _abs(rel)
        if not os.path.exists(path):
            rep.fail(f"B  {rel} does not exist on disk")
            continue
        with open(path, "rb") as fh:
            on_disk = fh.read()
        import hashlib
        actual = hashlib.sha256(on_disk).hexdigest()
        if actual != digest:
            rep.fail(
                f"B  {rel} is stale\n"
                f"      on disk {actual[:16]}  fresh {digest[:16]}")
        else:
            rep.ok()

    # ---------------------------------------------------------------- C
    declared: set[str] = set(produced)
    for key in ("svg", "tsx", "dart"):
        declared.add(spec["emittedSources"][key]["path"])

    guard_dirs = [
        "apps/desktop/src-tauri/icons",
        "apps/desktop/public",
        "apps/mobile/assets",
        "apps/mobile/ios/Runner/Assets.xcassets/AppIcon.appiconset",
        "apps/mobile/android/app/src/main/res",
    ]
    image_ext = (".png", ".ico", ".icns", ".svg", ".jpg", ".jpeg", ".webp")
    for rel_dir in guard_dirs:
        base = _abs(rel_dir)
        if not os.path.isdir(base):
            continue
        for dirpath, _dirs, files in os.walk(base):
            for name in files:
                if not name.lower().endswith(image_ext):
                    continue
                full = os.path.join(dirpath, name)
                rel = os.path.relpath(full, REPO).replace(os.sep, "/")
                if rel in declared:
                    continue
                if name in ("generate_icons.py",):
                    continue
                rep.fail(
                    f"C  orphan image not produced by the spec: {rel}\n"
                    f"      add it to assets/brand/conduit.mark.json or delete it")
                rep.checks += 1

    # ---------------------------------------------------------------- D
    for entry in groups.get("tauriIco", []):
        rel = entry["path"]
        data = _read(rel)
        count = struct.unpack("<H", data[4:6])[0]
        want = list(entry["sizes"])
        if count != len(want):
            rep.fail(
                f"D  {rel} contains {count} frame(s), spec declares {len(want)} "
                f"({', '.join(str(s) for s in want)})\n"
                f"      Pillow drops any ICO frame larger than the base image, so "
                f"the base must be the largest render.")
            continue
        rep.ok()
        frames: dict[int, bytes] = {}
        for i in range(count):
            off = 6 + i * 16
            w, h = data[off], data[off + 1]
            nbytes, offset = struct.unpack("<II", data[off + 8:off + 16])
            frames[w if w else 256] = data[offset:offset + nbytes]
        for size in want:
            if size not in frames:
                rep.fail(f"D  {rel} has no {size}px frame")
                continue
            rep.ok()
            standalone = produced_png(spec, mark, "plate", size)
            if frames[size] != standalone:
                rep.fail(
                    f"D  {rel} {size}px frame payload is not byte-identical to the "
                    f"standalone {size}px render")

    for entry in groups.get("tauriIcns", []):
        rel = entry["path"]
        data = _read(rel)
        magic, total = struct.unpack(">4sI", data[:8])
        if magic != b"icns" or total != len(data):
            rep.fail(f"D  {rel} header declares {total} bytes, file is {len(data)}")
            continue
        rep.ok()
        pos, found = 8, []
        while pos < len(data):
            ctype = data[pos:pos + 4]
            clen = struct.unpack(">I", data[pos + 4:pos + 8])[0]
            if clen < 8 or pos + clen > len(data):
                rep.fail(f"D  {rel} chunk {ctype!r} at {pos} has bad length {clen}")
                break
            found.append(ctype)
            pos += clen
        else:
            rep.ok()
        want_types = [ICNS_TYPES[s] for s in entry["sizes"]]
        if found != want_types:
            rep.fail(
                f"D  {rel} chunk types {found} != declared {want_types}")

    # ---------------------------------------------------------------- E
    for group in groups.values():
        if not isinstance(group, list):
            continue
        for entry in group:
            if "size" not in entry:
                continue
            variant = variants[entry["variant"]]
            data = _read(entry["path"])
            if _png_size(data) != (entry["size"], entry["size"]):
                rep.fail(f"E  {entry['path']} is not {entry['size']}px square")
                continue
            rep.ok()
            corner = _png_corner_alpha(data)
            if variant["opaque"]:
                if corner != 255:
                    rep.fail(
                        f"E  {entry['path']} variant '{entry['variant']}' must be "
                        f"fully opaque (corner alpha {corner}, expected 255). "
                        f"iOS rejects alpha in AppIcon PNGs and the Android launcher "
                        f"applies its own mask.")
                else:
                    rep.ok()
                # An opaque PNG cannot show antialiasing in its alpha channel, so
                # prove the rasteriser antialiased by rendering the same geometry
                # on transparency and looking for partial alpha there. This is the
                # check that would have caught the old generator, which drew
                # straight to the final size and emitted alpha {0, 255} only.
                probe = render(spec, mark, "mark", entry["size"])
                buf = io.BytesIO()
                probe.save(buf, format="PNG")
                if not _partial_alpha(buf.getvalue()):
                    rep.fail(
                        f"E  {entry['path']}: the rasteriser produced no partial "
                        f"alpha on the mark edge - every edge is a hard staircase")
                else:
                    rep.ok()
            else:
                if variant["plate"] and corner != 0:
                    rep.fail(
                        f"E  {entry['path']} variant '{entry['variant']}' must have "
                        f"transparent rounded corners (corner alpha {corner}, expected 0)")
                else:
                    rep.ok()
                if entry["size"] >= 32 and not _partial_alpha(data):
                    rep.fail(
                        f"E  {entry['path']} has no partial alpha at all - the "
                        f"rasteriser is not antialiasing, so every edge is a hard "
                        f"staircase")
                else:
                    rep.ok()

    # ---------------------------------------------------------------- F
    grid = spec["grid"]
    width = spec["mark"]["tubeWidth"]
    radius = spec["mark"]["jointRadius"]
    centre = [(p["x"], p["y"]) for p in spec["mark"]["centerline"]]

    svg = _read(spec["emittedSources"]["svg"]["path"]).decode("utf-8")
    tsx = _read(spec["emittedSources"]["tsx"]["path"]).decode("utf-8")
    dart = _read(spec["emittedSources"]["dart"]["path"]).decode("utf-8")

    # The SVG has no named constants - it has baked coordinates. So verify it
    # geometrically: rebuild the tube from its own polygon/circle elements and
    # compare that against the spec. This is a real check, not a proxy.
    svg_grid = float(spec["emittedSources"]["svg"]["grid"])
    got = _svg_geometry(svg)
    if got is None:
        rep.fail("F  logo.svg: could not read <polygon>/<circle> geometry back out")
    else:
        g_pts, g_joints = got
        s = svg_grid / mark.grid
        want_pts = []
        for (x0, y0), (x1, y1), w in mark.segments():
            h = w / 2.0
            dx, dy = x1 - x0, y1 - y0
            ln = math.hypot(dx, dy)
            nx, ny = -dy / ln * h, dx / ln * h
            want_pts.append([(x0 + nx, y0 + ny), (x1 + nx, y1 + ny),
                             (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)])
        want_joints = [(cx, cy, r) for cx, cy, r in mark.joints()]
        bad = (len(g_pts) != len(want_pts)
               or len(g_joints) != len(want_joints)
               or any(abs(a[0] - b[0] * s) > 0.01 or abs(a[1] - b[1] * s) > 0.01
                      for got_poly, want_poly in zip(g_pts, want_pts)
                      for a, b in zip(got_poly, want_poly))
               or any(abs(g[0] - c * s) > 0.01 or abs(g[1] - d * s) > 0.01
                      or abs(g[2] - e * s) > 0.01
                      for g, (c, d, e) in zip(g_joints, want_joints)))
        if bad:
            rep.fail(
                f"F  logo.svg geometry drifted from conduit.mark.json "
                f"({len(g_pts)} polygons / {len(g_joints)} circles, spec has "
                f"{len(want_pts)} / {len(want_joints)})")
        else:
            rep.ok()

    for label, text, want_tube, want_radius, want_pts in (
        ("ConduitLogo.tsx", tsx, width, radius, centre),
        ("conduit_logo.dart", dart, width, radius, centre),
    ):
        nums = _extract_constants(text)
        if nums is None:
            rep.fail(f"F  {label}: could not read the generated constants back")
            continue
        g, w, r, pts = nums
        if abs(g - grid) > 1e-9 or abs(w - want_tube) > 1e-9 \
                or abs(r - want_radius) > 1e-9 or len(pts) != len(want_pts) \
                or any(abs(a[0] - b[0]) > 1e-6 or abs(a[1] - b[1]) > 1e-6
                       for a, b in zip(pts, want_pts)):
            rep.fail(
                f"F  {label} geometry drifted from conduit.mark.json\n"
                f"      file: grid={g} tube={w} joint={r} centreline={pts}\n"
                f"      spec: grid={grid} tube={want_tube} joint={want_radius} "
                f"centreline={want_pts}")
        else:
            rep.ok()

    for label, text in (("ConduitLogo.tsx", tsx), ("conduit_logo.dart", dart)):
        if spec["palette"]["accent"].lstrip("#").lower() not in text.lower():
            rep.fail(
                f"F  {label} no longer references the accent "
                f"#{spec['palette']['accent']} from the spec palette")
        else:
            rep.ok()
        if "feGaussianBlur" in text or "MaskFilter" in text or "blur(" in text:
            rep.fail(
                f"F  {label} reintroduced a blur/glow. Filter rasterisation is the "
                f"one thing in this pipeline that is not reproducible across "
                f"GPU/software paths, so the mark stays flat.")
        else:
            rep.ok()

    # ---------------------------------------------------------------- G
    tauri_path = _abs("apps/desktop/src-tauri/tauri.conf.json")
    with open(tauri_path, "r", encoding="utf-8") as fh:
        tauri = json.load(fh)
    for rel in tauri.get("bundle", {}).get("icon", []):
        key = f"apps/desktop/src-tauri/{rel}"
        if key not in declared:
            rep.fail(f"G  tauri.conf.json bundles {rel}, which the spec does not produce")
        else:
            rep.ok()

    # The tray icon is the one asset that CANNOT be the same pixels on every
    # desktop: macOS renders an NSStatusItem template image from its alpha
    # channel only and discards RGB, so a full-colour icon with a plate would
    # show up as a filled rounded rectangle. Tauri merges
    # tauri.<platform>.conf.json over tauri.conf.json, so macOS takes the
    # template asset and Windows/Linux take the full-colour one.
    tray = tauri.get("app", {}).get("trayIcon", {})
    tray_rel = tray.get("iconPath")
    if tray_rel:
        key = f"apps/desktop/src-tauri/{tray_rel}"
        if key not in declared:
            rep.fail(
                f"G  tauri.conf.json trayIcon.iconPath is {tray_rel}, which the "
                f"spec does not produce.")
        else:
            rep.ok()
        tray_entry = _entry_for(declared, key)
        tray_variant = spec["variants"].get(tray_entry or "", {})
        if tray.get("iconAsTemplate"):
            if tray_variant.get("plate") is not False:
                rep.fail(
                    f"G  trayIcon has iconAsTemplate: true but {tray_rel} is not a "
                    f"plateless variant. macOS discards RGB for template images, so "
                    f"an icon with a plate renders as a solid block in the menu bar.")
            else:
                rep.ok()
        else:
            rep.fail(
                f"G  trayIcon must be iconAsTemplate on macOS so the mark follows "
                f"the menu-bar foreground colour. Remove the flag only if the base "
                f"config is macOS-only.")
        for platform in ("windows", "linux"):
            override = f"apps/desktop/src-tauri/tauri.{platform}.conf.json"
            if not os.path.exists(_abs(override)):
                rep.fail(
                    f"G  missing {platform} tray override. iconAsTemplate is honoured "
                    f"only on macOS; without an override, Windows and Linux would "
                    f"composite a monochrome template image as a black blob.")
                continue
            with open(_abs(override), "r", encoding="utf-8") as fh:
                ov = json.load(fh)
            ov_tray = ov.get("app", {}).get("trayIcon", {})
            ov_key = f"apps/desktop/src-tauri/{ov_tray.get('iconPath', '')}"
            if ov_tray.get("iconAsTemplate") is not False:
                rep.fail(
                    f"G  {platform} tray override must set iconAsTemplate: false")
            elif ov_key not in declared:
                rep.fail(
                    f"G  {platform} tray override points at "
                    f"{ov_tray.get('iconPath')}, which the spec does not produce")
            else:
                rep.ok()

    with open(_abs("apps/desktop/index.html"), "r", encoding="utf-8") as fh:
        html = fh.read()
    # index.html references assets by their path inside public/, so map back.
    public_prefix = "apps/desktop/public/"
    for href in re.findall(r'href="/([^"]+\.png)"', html):
        target = public_prefix + href.lstrip("/")
        if target not in declared:
            rep.fail(f"G  index.html references /{href}, which the spec does not produce")
        else:
            rep.ok()

    manifest_path = _abs("apps/mobile/android/app/src/main/AndroidManifest.xml")
    with open(manifest_path, "r", encoding="utf-8") as fh:
        manifest_xml = fh.read()
    for ref in re.findall(r'android:icon="@mipmap/([^"]+)"', manifest_xml):
        # AndroidManifest names a resource, not a file, so compare basenames.
        produced_names = {os.path.splitext(os.path.basename(p))[0]
                          for p in declared if "/mipmap" in p}
        if ref not in produced_names:
            rep.fail(
                f"G  AndroidManifest.xml points android:icon at @mipmap/{ref}, "
                f"which the spec does not produce. The spec produces: "
                f"{', '.join(sorted(produced_names))}")
        else:
            rep.ok()

    # ---------------------------------------------------------------- done
    print()
    print(f"icon drift  mark v{spec['version']}  tube {width:g}u  "
          f"centreline {centre}")
    print(f"  spec        assets/brand/conduit.mark.json")
    print(f"  files       {len(declared)} tracked, {rep.checks} assertions")
    if rep.problems:
        print()
        for p in rep.problems:
            print(f"  FAIL  {p}")
        print()
        print(f"  {len(rep.problems)} problem(s). Run 'npm run icons' in apps/desktop "
              f"and commit the result.")
        return 1
    print(f"  0 problems - every asset is a fresh render of the spec")
    return 0


def produced_png(spec: dict, mark, variant: str, size: int) -> bytes:
    from build_icons import render
    buf = io.BytesIO()
    render(spec, mark, variant, size).save(buf, format="PNG")
    return buf.getvalue()


def _svg_geometry(text: str):
    """Rebuild (polygons, circles) from a generated SVG."""
    polys = []
    for chunk in re.findall(r"<polygon[^>]*points=\"([^\"]+)\"", text):
        nums = [float(v) for v in re.split(r"[ ,]+", chunk.strip()) if v]
        polys.append([(nums[i], nums[i + 1]) for i in range(0, len(nums), 2)])
    circles = []
    for blob in re.findall(r"<circle[^>]*/?>", text):
        cx = re.search(r'cx="([-\d.]+)"', blob)
        cy = re.search(r'cy="([-\d.]+)"', blob)
        r = re.search(r'r="([-\d.]+)"', blob)
        if cx and cy and r:
            circles.append((float(cx.group(1)), float(cy.group(1)), float(r.group(1))))
    return polys, circles


_PAIR = re.compile(r"(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)")


def _extract_constants(text: str):
    """Read grid / tube width / joint radius / centreline back out of a
    generated file, so we compare the *emitted* values and not our own memory."""
    grid = re.search(r"GRID\s*=\s*([\d.]+)|kGrid\s*=\s*([\d.]+)", text)
    tube = re.search(r"TUBE_WIDTH\s*=\s*([\d.]+)|kTubeWidth\s*=\s*([\d.]+)", text)
    joint = re.search(r"JOINT_RADIUS\s*=\s*([\d.]+)|kJointRadius\s*=\s*([\d.]+)", text)
    if not (grid and tube and joint):
        return None
    g = float(grid.group(1) or grid.group(2))
    w = float(tube.group(1) or tube.group(2))
    r = float(joint.group(1) or joint.group(2))

    block = re.search(
        r"(?:CENTERLINE|kCenterline)\s*(?::[^=]*)?=\s*(?:<Offset>)?\[(.*?)\];",
        text, re.S)
    if not block:
        return None
    pts = [(float(a), float(b)) for a, b in _PAIR.findall(block.group(1))]
    return g, w, r, pts


if __name__ == "__main__":
    import build_icons
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    sys.exit(build_icons.main())
