#!/usr/bin/env python3
"""Compare generated brand assets against the committed versions, by content.

Why this exists
---------------
The obvious check is `git diff --exit-code` after regenerating. That works for
text outputs, and it is wrong for the images.

PNG's IDAT chunk is a zlib deflate stream. The pixels are identical, but the
*compressed bytes* are a function of the zlib build that produced them - and
Pillow's Windows and manylinux wheels statically link different ones. So a
render on Windows and a render on the same commit in CI produce the same image
and different files, and a byte-level diff reports drift that is not there.
That is what made the "icon drift" job fail on its first run, on every image,
with a clean report from verify_icons.py beside it.

So: compare what the asset *is*, not how it happens to be stored. For images,
decode both sides and compare the pixel buffers. For everything else, compare
bytes. Text is byte-reproducible; compressed rasters are not.

Exit codes
----------
0  no drift
1  real drift - a committed asset differs from a fresh render
2  a listed asset is missing, or the tree is not a git repository

Usage
-----
    python scripts/icons/compare_generated.py [--against REF]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    sys.exit("Pillow is required: pip install -r scripts/icons/requirements.txt")

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
MANIFEST = os.path.join(REPO, "assets", "brand", "icon-manifest.json")
RENDERABLE = (".png",)


def git(*args: str) -> str:
    return subprocess.run(
        ("git",) + args, cwd=REPO, check=True, capture_output=True, text=True
    ).stdout


def committed_bytes(path: str, ref: str) -> bytes:
    """Read `path` as recorded in `ref`, or None if it did not exist there."""
    proc = subprocess.run(
        ("git", "show", f"{ref}:{path}"),
        cwd=REPO,
        capture_output=True,
    )
    return proc.stdout if proc.returncode == 0 else None


def pixels(data: bytes) -> tuple:
    """Decode to a comparable form: mode, size and the raw buffer.

    Normalising the mode matters. A PNG can decode to P, RGBA or RGB for what
    is visually the same artwork depending on how it was written, and comparing
    the raw buffers of two different modes would report drift that a viewer
    cannot see. Converting to RGBA makes the comparison mean "does this look
    the same".
    """
    import io

    with Image.open(io.BytesIO(data)) as img:
        return (img.size, img.convert("RGBA").tobytes())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--against",
        default="HEAD",
        help="git ref holding the committed assets (default: HEAD)",
    )
    args = ap.parse_args()

    if not os.path.isdir(os.path.join(REPO, ".git")):
        sys.exit("compare_generated.py must run inside the repository")

    with open(MANIFEST, encoding="utf-8") as fh:
        entries = json.load(fh)["files"]

    compared = skipped = 0
    problems: list[str] = []
    missing: list[str] = []

    for path in sorted(entries):
        absolute = os.path.join(REPO, path)
        if not os.path.exists(absolute):
            missing.append(f"{path}: not present in the working tree")
            continue

        old = committed_bytes(path, args.against)
        if old is None:
            skipped += 1
            continue

        with open(absolute, "rb") as fh:
            new = fh.read()

        if old == new:
            compared += 1
            continue

        # Bytes differ. If it is a renderable image, decide whether the
        # artwork changed or only the encoding did.
        if path.lower().endswith(RENDERABLE):
            try:
                if pixels(old) == pixels(new):
                    compared += 1  # same image, different deflate stream
                    continue
            except Exception as exc:  # noqa: BLE001 - report and treat as drift
                problems.append(f"{path}: could not decode ({exc})")
                continue

        problems.append(
            f"{path}: committed and regenerated assets differ "
            f"(run 'npm run icons' in apps/desktop and commit the result)"
        )

    for note in missing:
        print(f"  MISSING  {note}", file=sys.stderr)

    print(
        f"  {compared} of {len(entries)} assets match the committed version "
        f"({skipped} new, not in {args.against})"
    )

    if problems:
        print(f"\n  {len(problems)} asset(s) have real drift:", file=sys.stderr)
        for note in problems:
            print(f"    {note}", file=sys.stderr)
        return 1

    if missing:
        return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
