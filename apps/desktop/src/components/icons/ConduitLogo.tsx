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

import type { FC } from 'react';

const GRID = 64;
const TUBE_WIDTH = 15;
const JOINT_RADIUS = 7.5;

const CENTERLINE: ReadonlyArray<readonly [number, number]> = [
  [2, 14],
  [26, 14],
  [38, 48],
  [62, 48]
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
  color = 'var(--accent, #00F0FF)',
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
