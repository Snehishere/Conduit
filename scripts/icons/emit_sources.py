#!/usr/bin/env python3
"""Emit the generated code files. Split out of build_icons.py for readability.

Code is emitted with %%TOKEN%% substitution rather than str.format, because the
templates are full of braces and escaping them is a bug factory.
"""
from __future__ import annotations

import math

HEADER_TSX = """// GENERATED FILE - DO NOT EDIT.
// Source of truth: assets/brand/conduit.mark.json
// Regenerate:      npm run icons      (run from apps/desktop)
// Verify:          npm run icons:verify
//
// The conduit is one unbroken channel of constant thickness, swept along a
// polyline. Its silhouette is the union of one rotated rectangle per segment
// plus a disc at each interior joint. The two terminals get no disc, so they
// stay flat sections cut perpendicular to the axis - that is what makes the
// mark read as a sectioned pipe rather than as a letterform.
"""

TSX_BODY = """
import type { FC } from 'react';

const GRID = %%GRID%%;
const TUBE_WIDTH = %%TUBE%%;
const JOINT_RADIUS = %%JOINT%%;

const CENTERLINE: ReadonlyArray<readonly [number, number]> = [
%%CENTRELINE%%
];

interface ConduitSegment {
  ax: number;
  ay: number;
  bx: number;
  by: number;
  nx: number;
  ny: number;
}

const SEGMENTS: ReadonlyArray<ConduitSegment> = (() => {
  const h = TUBE_WIDTH / 2;
  const out: ConduitSegment[] = [];
  for (let i = 0; i < CENTERLINE.length - 1; i++) {
    const [ax, ay] = CENTERLINE[i];
    const [bx, by] = CENTERLINE[i + 1];
    const dx = bx - ax;
    const dy = by - ay;
    const len = Math.hypot(dx, dy) || 1;
    out.push({ ax, ay, bx, by, nx: (-dy / len) * h, ny: (dx / len) * h });
  }
  return out;
})();

const JOINTS: ReadonlyArray<readonly [number, number, number]> = CENTERLINE.slice(
  1,
  -1,
).map(([x, y]) => [x, y, JOINT_RADIUS] as const);

export interface ConduitLogoProps {
  /** Rendered edge length in CSS pixels. The mark occupies 60/64 of this. */
  size?: number;
  className?: string;
  /** Draw the dark plate behind the mark. Used by the splash screen. */
  withBackground?: boolean;
  /** Defaults to the app accent token so the mark follows the user's theme. */
  color?: string;
}

/** The Conduit mark: a sectioned pipe offset. Shared by header, splash and dock. */
const ConduitLogo: FC<ConduitLogoProps> = ({
  size = 16,
  className,
  withBackground = false,
  color = 'var(--accent, %%ACCENT_CSS%%)',
}) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox={`0 0 ${GRID} ${GRID}`}
    width={size}
    height={size}
    className={className}
    role="img"
    aria-label="Conduit"
    style={{ flexShrink: 0 }}
  >
    {withBackground && (
      <rect width={GRID} height={GRID} rx={Math.round(GRID * 0.216)} fill="#0B0C10" />
    )}
    <g fill={color}>
      {SEGMENTS.map((s, i) => (
        <polygon
          key={`conduit-segment-${i}`}
          points={`${s.ax + s.nx},${s.ay + s.ny} ${s.bx + s.nx},${s.by + s.ny} ${s.bx - s.nx},${s.by - s.ny} ${s.ax - s.nx},${s.ay - s.ny}`}
        />
      ))}
      {JOINTS.map(([cx, cy, r], i) => (
        <circle key={`conduit-joint-${i}`} cx={cx} cy={cy} r={r} />
      ))}
    </g>
  </svg>
);

export default ConduitLogo;
"""

HEADER_DART = """// GENERATED FILE - DO NOT EDIT.
// Source of truth: assets/brand/conduit.mark.json
// Regenerate:      npm run icons      (run from apps/desktop)
// Verify:          npm run icons:verify
//
// The conduit is one unbroken channel of constant thickness, swept along a
// polyline. Its silhouette is the union of one rotated rectangle per segment
// plus a disc at each interior joint. The two terminals get no disc, so they
// stay flat sections cut perpendicular to the axis - that is what makes the
// mark read as a sectioned pipe rather than as a letterform.
"""

DART_BODY = """
import 'package:flutter/material.dart';

const double kGrid = %%GRID%%;
const double kTubeWidth = %%TUBE%%;
const double kJointRadius = %%JOINT%%;

const List<Offset> kCenterline = <Offset>[
%%CENTRELINE%%,
];
class _ConduitSegment {
  final Offset a;
  final Offset b;
  final Offset n;

  const _ConduitSegment(this.a, this.b, this.n);
}

final List<_ConduitSegment> _segments = () {
  const h = kTubeWidth / 2;
  final out = <_ConduitSegment>[];
  for (var i = 0; i < kCenterline.length - 1; i++) {
    final a = kCenterline[i];
    final b = kCenterline[i + 1];
    final d = b - a;
    final len = d.distance == 0 ? 1.0 : d.distance;
    out.add(_ConduitSegment(a, b, Offset(-d.dy / len * h, d.dx / len * h)));
  }
  return out;
}();

class _ConduitJoint {
  final Offset c;
  final double r;

  const _ConduitJoint(this.c, this.r);
}

final List<_ConduitJoint> _joints = <_ConduitJoint>[
  for (final p in kCenterline.sublist(1, kCenterline.length - 1))
    _ConduitJoint(p, kJointRadius),
];

/// The Conduit mark: a sectioned pipe offset. Shared by the mobile shell.
class ConduitLogo extends StatelessWidget {
  final double size;
  final Color? color;

  const ConduitLogo({super.key, this.size = 24, this.color});

  @override
  Widget build(BuildContext context) => SizedBox(
    width: size,
    height: size,
    child: CustomPaint(painter: _ConduitLogoPainter(color: color)),
  );
}
class _ConduitLogoPainter extends CustomPainter {
  final Color? color;

  const _ConduitLogoPainter({this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.width / kGrid;
    final paint = Paint()
      ..style = PaintingStyle.fill
      ..color = color ?? const Color(0xFF%%ACCENT%%);
    canvas
      ..save()
      ..scale(scale);
    for (final s in _segments) {
      canvas.drawPath(
        Path()
          ..moveTo(s.a.dx + s.n.dx, s.a.dy + s.n.dy)
          ..lineTo(s.b.dx + s.n.dx, s.b.dy + s.n.dy)
          ..lineTo(s.b.dx - s.n.dx, s.b.dy - s.n.dy)
          ..lineTo(s.a.dx - s.n.dx, s.a.dy - s.n.dy)
          ..close(),
        paint,
      );
    }
    for (final j in _joints) {
      canvas.drawCircle(j.c, j.r, paint);
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _ConduitLogoPainter oldDelegate) =>
      oldDelegate.color != color;
}
"""


def fmt(n: float, places: int = 3) -> str:
    text = f"{n:.{places}f}".rstrip("0").rstrip(".")
    return text if text not in ("", "-0") else "0"


def _tokens(mark, accent_css: str, accent_bare: str, centreline_block: str) -> dict[str, str]:
    return {
        "%%GRID%%": fmt(mark.grid),
        "%%TUBE%%": fmt(mark.width),
        "%%JOINT%%": fmt(mark.joint_radius),
        "%%ACCENT%%": accent_bare,
        "%%ACCENT_CSS%%": accent_css,
        "%%CENTRELINE%%": centreline_block,
    }


def _substitute(body: str, tokens: dict[str, str]) -> str:
    # Two-phase so %%ACCENT_CSS%% is not clobbered by a %%ACCENT%% prefix match.
    for key in sorted(tokens, key=len, reverse=True):
        body = body.replace(key, tokens[key])
    if "%%" in body:
        tail = body[body.index("%%"):body.index("%%") + 20]
        raise SystemExit(f"unsubstituted token left in template: {tail!r}")
    return body


def tsx_for(mark, accent: str) -> str:
    block = ",\n".join(f"  [{fmt(x)}, {fmt(y)}]" for x, y in mark.centerline)
    return HEADER_TSX + _substitute(
        TSX_BODY, _tokens(mark, accent, accent, block))


def dart_for(mark, accent: str) -> str:
    block = ",\n".join(f"  Offset({fmt(x)}, {fmt(y)})" for x, y in mark.centerline)
    return HEADER_DART + _substitute(
        DART_BODY, _tokens(mark, accent, accent.lstrip("#"), block))


def svg_for(spec: dict, mark, grid: int) -> str:
    pal = spec["palette"]
    k = grid / mark.grid
    variant = spec["variants"]["plate"]
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {grid} {grid}" '
        f'width="{grid}" height="{grid}" role="img" aria-label="Conduit">',
        "  <title>Conduit</title>",
        "  <!-- GENERATED from assets/brand/conduit.mark.json by "
        "scripts/icons/build_icons.py. Do not edit. -->",
        f'  <defs><linearGradient id="conduit-plate" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{pal["plateTop"]}"/>'
        f'<stop offset="1" stop-color="{pal["plateBottom"]}"/>'
        f'</linearGradient></defs>',
        f'  <rect width="{grid}" height="{grid}" rx="{fmt(variant["cornerRadius"] * grid)}" '
        f'fill="url(#conduit-plate)"/>',
    ]
    colour = pal[variant["markColor"]]
    for (x0, y0), (x1, y1), w in mark.segments():
        dx, dy = x1 - x0, y1 - y0
        ln = math.hypot(dx, dy)
        h = w / 2.0
        nx, ny = -dy / ln * h, dx / ln * h
        pts = " ".join(
            f"{fmt(px * k)},{fmt(py * k)}"
            for px, py in ((x0 + nx, y0 + ny), (x1 + nx, y1 + ny),
                           (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)))
        out.append(f'  <polygon points="{pts}" fill="{colour}"/>')
    for cx, cy, r in mark.joints():
        out.append(f'  <circle cx="{fmt(cx * k)}" cy="{fmt(cy * k)}" '
                   f'r="{fmt(r * k)}" fill="{colour}"/>')
    out.append("</svg>")
    return "\n".join(out) + "\n"
