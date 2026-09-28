// GENERATED FILE - DO NOT EDIT.
// Source of truth: assets/brand/conduit.mark.json
// Regenerate:      npm run icons      (run from apps/desktop)
// Verify:          npm run icons:verify
//
// The conduit is one unbroken channel of constant thickness, swept along a
// polyline. Its silhouette is the union of one rotated rectangle per segment
// plus a disc at each interior joint. The two terminals get no disc, so they
// stay flat sections cut perpendicular to the axis - that is what makes the
// mark read as a sectioned pipe rather than as a letterform.

import 'package:flutter/material.dart';

const double kGrid = 64;
const double kTubeWidth = 15;
const double kJointRadius = 7.5;

const List<Offset> kCenterline = <Offset>[
  Offset(2, 14),
  Offset(26, 14),
  Offset(38, 48),
  Offset(62, 48),
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
      ..color = color ?? const Color(0xFF00F0FF);
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
