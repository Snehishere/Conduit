import React from 'react';

interface BubbleProps {
  id: string;
  x: number;
  y: number;
  radius: number;
  scale: number;
  opacity: number;
  popping: boolean;
  name: string;
  deviceType: string;
  os: string;
  battery?: number;
  status: string;
  isSelected: boolean;
  isDragging: boolean;
}

/* ─── 3D Device Models ─── */

const DevicePhone: React.FC<{ s: number }> = ({ s }) => {
  const w = s * 0.38;
  const h = s * 0.7;
  return (
    <div style={{ width: w, height: h, position: 'relative', transformStyle: 'preserve-3d', transform: 'rotateX(-12deg) rotateY(18deg)' }}>
      {/* Body */}
      <div style={{
        width: '100%', height: '100%', borderRadius: w * 0.18,
        background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 40%, #1a1c22 100%)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '4px 6px 20px rgba(0,0,0,0.6), -1px -1px 0 rgba(255,255,255,0.06)',
        position: 'relative', overflow: 'hidden',
      }}>
        {/* Screen */}
        <div style={{
          position: 'absolute', top: '8%', left: '8%', right: '8%', bottom: '8%',
          borderRadius: w * 0.1,
          background: 'linear-gradient(180deg, #0c2818 0%, #0a1f14 50%, #0c2818 100%)',
          boxShadow: 'inset 0 0 12px rgba(52,211,153,0.08)',
        }}>
          {/* Screen content lines */}
          <div style={{ padding: '12% 10%' }}>
            <div style={{ width: '60%', height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.12)', marginBottom: '8%' }} />
            <div style={{ width: '80%', height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.06)', marginBottom: '8%' }} />
            <div style={{ width: '45%', height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.06)', marginBottom: '12%' }} />
            <div style={{ display: 'flex', gap: '6%' }}>
              <div style={{ width: '28%', height: 16, borderRadius: 3, background: 'rgba(52,211,153,0.12)' }} />
              <div style={{ width: '28%', height: 16, borderRadius: 3, background: 'rgba(255,255,255,0.04)' }} />
            </div>
          </div>
        </div>
        {/* Notch */}
        <div style={{
          position: 'absolute', top: '3.5%', left: '50%', transform: 'translateX(-50%)',
          width: '28%', height: 4, borderRadius: 3, background: 'rgba(0,0,0,0.6)',
        }} />
        {/* Side button */}
        <div style={{
          position: 'absolute', right: -1, top: '25%', width: 2, height: '12%',
          borderRadius: '0 1px 1px 0', background: 'rgba(255,255,255,0.08)',
        }} />
      </div>
    </div>
  );
};

const DeviceDesktop: React.FC<{ s: number }> = ({ s }) => {
  const w = s * 0.72;
  const h = s * 0.52;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', transformStyle: 'preserve-3d', transform: 'rotateX(-8deg) rotateY(12deg)' }}>
      {/* Monitor */}
      <div style={{
        width: w, height: h, borderRadius: w * 0.04,
        background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 40%, #1a1c22 100%)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '4px 6px 20px rgba(0,0,0,0.6), -1px -1px 0 rgba(255,255,255,0.06)',
        position: 'relative', overflow: 'hidden',
      }}>
        {/* Screen */}
        <div style={{
          position: 'absolute', top: '6%', left: '4%', right: '4%', bottom: '6%',
          borderRadius: w * 0.02,
          background: 'linear-gradient(180deg, #0c1a28 0%, #0a1520 50%, #0c1a28 100%)',
          boxShadow: 'inset 0 0 12px rgba(52,211,153,0.06)',
        }}>
          {/* Window chrome */}
          <div style={{ padding: '8% 6%' }}>
            <div style={{ display: 'flex', gap: 3, marginBottom: '8%' }}>
              <div style={{ width: 3, height: 3, borderRadius: '50%', background: 'rgba(255,100,100,0.4)' }} />
              <div style={{ width: 3, height: 3, borderRadius: '50%', background: 'rgba(255,200,60,0.4)' }} />
              <div style={{ width: 3, height: 3, borderRadius: '50%', background: 'rgba(100,220,100,0.4)' }} />
            </div>
            <div style={{ width: '100%', height: '50%', borderRadius: 2, background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.04)' }}>
              <div style={{ padding: '6%' }}>
                <div style={{ width: '40%', height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.1)', marginBottom: '8%' }} />
                <div style={{ width: '70%', height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.05)' }} />
              </div>
            </div>
          </div>
        </div>
        {/* Apple/logo area */}
        <div style={{
          position: 'absolute', bottom: '2%', left: '50%', transform: 'translateX(-50%)',
          width: 4, height: 4, borderRadius: '50%', background: 'rgba(255,255,255,0.06)',
        }} />
      </div>
      {/* Neck */}
      <div style={{
        width: w * 0.06, height: s * 0.06,
        background: 'linear-gradient(180deg, rgba(255,255,255,0.08), rgba(255,255,255,0.04))',
      }} />
      {/* Base */}
      <div style={{
        width: w * 0.3, height: 3, borderRadius: 2,
        background: 'rgba(255,255,255,0.06)',
        boxShadow: '0 2px 6px rgba(0,0,0,0.3)',
      }} />
    </div>
  );
};

const DeviceTablet: React.FC<{ s: number }> = ({ s }) => {
  const w = s * 0.6;
  const h = s * 0.46;
  return (
    <div style={{ width: w, height: h, transformStyle: 'preserve-3d', transform: 'rotateX(-10deg) rotateY(15deg)' }}>
      <div style={{
        width: '100%', height: '100%', borderRadius: w * 0.06,
        background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 40%, #1a1c22 100%)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '4px 6px 20px rgba(0,0,0,0.6), -1px -1px 0 rgba(255,255,255,0.06)',
        position: 'relative', overflow: 'hidden',
      }}>
        {/* Screen */}
        <div style={{
          position: 'absolute', top: '7%', left: '6%', right: '6%', bottom: '7%',
          borderRadius: w * 0.03,
          background: 'linear-gradient(180deg, #1a0c28 0%, #120a1f 50%, #1a0c28 100%)',
          boxShadow: 'inset 0 0 10px rgba(168,85,247,0.06)',
        }}>
          <div style={{ padding: '10% 8%' }}>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '5%' }}>
              {[0,1,2,3].map(i => (
                <div key={i} style={{ height: 14, borderRadius: 3, background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.03)' }} />
              ))}
            </div>
          </div>
        </div>
        {/* Camera */}
        <div style={{
          position: 'absolute', top: '3%', left: '50%', transform: 'translateX(-50%)',
          width: 3, height: 3, borderRadius: '50%', background: 'rgba(0,0,0,0.5)',
          border: '0.5px solid rgba(255,255,255,0.06)',
        }} />
      </div>
    </div>
  );
};

const DeviceWatch: React.FC<{ s: number }> = ({ s }) => {
  const w = s * 0.34;
  const h = s * 0.4;
  return (
    <div style={{ position: 'relative', width: w, height: h, transformStyle: 'preserve-3d', transform: 'rotateX(-10deg) rotateY(18deg)' }}>
      {/* Band top */}
      <div style={{
        position: 'absolute', top: 0, left: '28%', right: '28%', height: '22%',
        background: 'linear-gradient(180deg, rgba(60,60,70,0.6), rgba(45,45,55,0.8))',
        borderRadius: '3px 3px 0 0',
        border: '1px solid rgba(255,255,255,0.06)',
        borderBottom: 'none',
      }} />
      {/* Band bottom */}
      <div style={{
        position: 'absolute', bottom: 0, left: '28%', right: '28%', height: '22%',
        background: 'linear-gradient(0deg, rgba(60,60,70,0.6), rgba(45,45,55,0.8))',
        borderRadius: '0 0 3px 3px',
        border: '1px solid rgba(255,255,255,0.06)',
        borderTop: 'none',
      }} />
      {/* Body */}
      <div style={{
        position: 'absolute', top: '18%', left: 0, right: 0, bottom: '18%',
        borderRadius: w * 0.3,
        background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 40%, #1a1c22 100%)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '3px 4px 14px rgba(0,0,0,0.5), -1px -1px 0 rgba(255,255,255,0.06)',
        overflow: 'hidden',
      }}>
        {/* Screen */}
        <div style={{
          position: 'absolute', top: '12%', left: '12%', right: '12%', bottom: '12%',
          borderRadius: w * 0.18,
          background: 'linear-gradient(180deg, #0c1a18 0%, #0a1512 100%)',
        }}>
          {/* Time */}
          <div style={{ textAlign: 'center', paddingTop: '18%' }}>
            <div style={{ fontSize: s * 0.1, fontWeight: 700, color: 'rgba(255,255,255,0.7)', letterSpacing: 1 }}>10:42</div>
          </div>
          {/* Complications */}
          <div style={{ display: 'flex', justifyContent: 'space-around', padding: '8% 6%' }}>
            <div style={{ width: 5, height: 5, borderRadius: '50%', border: '1px solid rgba(52,211,153,0.3)' }} />
            <div style={{ width: 5, height: 5, borderRadius: '50%', border: '1px solid rgba(239,68,68,0.3)' }} />
          </div>
        </div>
      </div>
      {/* Crown */}
      <div style={{
        position: 'absolute', right: -3, top: '40%', width: 3, height: 5,
        borderRadius: 1, background: 'rgba(255,255,255,0.08)',
      }} />
    </div>
  );
};

const DeviceHeadphones: React.FC<{ s: number }> = ({ s }) => (
  <div style={{ position: 'relative', width: s * 0.56, height: s * 0.56, transformStyle: 'preserve-3d', transform: 'rotateX(-8deg) rotateY(10deg)' }}>
    {/* Headband */}
    <div style={{
      position: 'absolute', top: '5%', left: '8%', right: '8%', height: '30%',
      borderBottom: '2.5px solid rgba(255,255,255,0.12)',
      borderRadius: '0 0 50% 50%',
      boxShadow: '0 2px 4px rgba(0,0,0,0.3)',
    }} />
    {/* Left arm */}
    <div style={{
      position: 'absolute', left: '8%', top: '15%', width: 2, height: '40%',
      background: 'rgba(255,255,255,0.08)', borderRadius: 1,
    }} />
    {/* Right arm */}
    <div style={{
      position: 'absolute', right: '8%', top: '15%', width: 2, height: '40%',
      background: 'rgba(255,255,255,0.08)', borderRadius: 1,
    }} />
    {/* Left cup */}
    <div style={{
      position: 'absolute', left: 0, bottom: '8%', width: '35%', height: '38%',
      borderRadius: '35%', transform: 'rotate(-5deg)',
      background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 50%, #1a1c22 100%)',
      border: '1px solid rgba(255,255,255,0.10)',
      boxShadow: '3px 4px 12px rgba(0,0,0,0.5)',
    }}>
      <div style={{
        position: 'absolute', top: '20%', left: '20%', right: '20%', bottom: '20%',
        borderRadius: '50%', background: 'rgba(0,0,0,0.3)',
        border: '0.5px solid rgba(255,255,255,0.04)',
      }} />
    </div>
    {/* Right cup */}
    <div style={{
      position: 'absolute', right: 0, bottom: '8%', width: '35%', height: '38%',
      borderRadius: '35%', transform: 'rotate(5deg)',
      background: 'linear-gradient(200deg, #3a3d45 0%, #22242a 50%, #1a1c22 100%)',
      border: '1px solid rgba(255,255,255,0.10)',
      boxShadow: '3px 4px 12px rgba(0,0,0,0.5)',
    }}>
      <div style={{
        position: 'absolute', top: '20%', left: '20%', right: '20%', bottom: '20%',
        borderRadius: '50%', background: 'rgba(0,0,0,0.3)',
        border: '0.5px solid rgba(255,255,255,0.04)',
      }} />
    </div>
  </div>
);

const DeviceTV: React.FC<{ s: number }> = ({ s }) => {
  const w = s * 0.74;
  const h = s * 0.44;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', transformStyle: 'preserve-3d', transform: 'rotateX(-6deg) rotateY(8deg)' }}>
      <div style={{
        width: w, height: h, borderRadius: w * 0.025,
        background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 40%, #1a1c22 100%)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '4px 6px 24px rgba(0,0,0,0.6), -1px -1px 0 rgba(255,255,255,0.06)',
        position: 'relative', overflow: 'hidden',
      }}>
        <div style={{
          position: 'absolute', top: '4%', left: '3%', right: '3%', bottom: '4%',
          borderRadius: w * 0.01,
          background: 'linear-gradient(180deg, #0f0f1a 0%, #0a0a14 50%, #0f0f1a 100%)',
          boxShadow: 'inset 0 0 14px rgba(0,240,255,0.06)',
        }}>
          {/* App grid */}
          <div style={{ padding: '6% 5%', display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '4%' }}>
            {[0,1,2,3,4,5].map(i => (
              <div key={i} style={{ aspectRatio: '1', borderRadius: 3, background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.02)' }} />
            ))}
          </div>
        </div>
      </div>
      <div style={{ width: 4, height: s * 0.04, background: 'rgba(255,255,255,0.05)' }} />
      <div style={{ width: w * 0.25, height: 2, borderRadius: 1, background: 'rgba(255,255,255,0.05)' }} />
    </div>
  );
};

const DeviceEarbuds: React.FC<{ s: number }> = ({ s }) => (
  <div style={{ position: 'relative', width: s * 0.5, height: s * 0.45, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: s * 0.1, transformStyle: 'preserve-3d', transform: 'rotateX(-8deg) rotateY(12deg)' }}>
    {/* Left */}
    <div style={{
      width: '38%', height: '55%', borderRadius: '42% 42% 38% 38%',
      background: 'linear-gradient(160deg, #3a3d45 0%, #22242a 50%, #1a1c22 100%)',
      border: '1px solid rgba(255,255,255,0.10)',
      boxShadow: '2px 3px 10px rgba(0,0,0,0.5)',
      transform: 'rotate(-8deg)',
    }}>
      <div style={{ position: 'absolute', top: '30%', left: '25%', right: '25%', bottom: '30%', borderRadius: '50%', background: 'rgba(0,0,0,0.4)', border: '0.5px solid rgba(255,255,255,0.04)' }} />
      {/* Stem */}
      <div style={{ position: 'absolute', bottom: '-20%', left: '35%', width: '30%', height: '22%', borderRadius: '0 0 3px 3px', background: 'linear-gradient(180deg, #2a2d35, #22242a)', border: '1px solid rgba(255,255,255,0.06)', borderTop: 'none' }} />
    </div>
    {/* Right */}
    <div style={{
      width: '38%', height: '55%', borderRadius: '42% 42% 38% 38%',
      background: 'linear-gradient(200deg, #3a3d45 0%, #22242a 50%, #1a1c22 100%)',
      border: '1px solid rgba(255,255,255,0.10)',
      boxShadow: '2px 3px 10px rgba(0,0,0,0.5)',
      transform: 'rotate(8deg)',
    }}>
      <div style={{ position: 'absolute', top: '30%', left: '25%', right: '25%', bottom: '30%', borderRadius: '50%', background: 'rgba(0,0,0,0.4)', border: '0.5px solid rgba(255,255,255,0.04)' }} />
      <div style={{ position: 'absolute', bottom: '-20%', left: '35%', width: '30%', height: '22%', borderRadius: '0 0 3px 3px', background: 'linear-gradient(180deg, #2a2d35, #22242a)', border: '1px solid rgba(255,255,255,0.06)', borderTop: 'none' }} />
    </div>
  </div>
);

const DEVICE_MODELS: Record<string, React.FC<{ s: number }> | undefined> = {
  desktop: DeviceDesktop,
  laptop: DeviceDesktop,
  phone: DevicePhone,
  tablet: DeviceTablet,
  earbuds: DeviceEarbuds,
  headphones: DeviceHeadphones,
  watch: DeviceWatch,
  tv: DeviceTV,
};

/* ─── Main Bubble ─── */

const DeviceBubble: React.FC<BubbleProps> = ({
  x, y, radius, scale, opacity, popping,
  name, deviceType, isSelected, isDragging,
}) => {
  const DeviceModel = DEVICE_MODELS[deviceType] || DevicePhone;
  const size = radius * 2;
  // Suspended model: radius × 1.10 keeps the bounding box ≤62% of the diameter
  // with a visible gap between the model shadow and the sphere wall (§B2).
  const modelSize = radius * 1.1;

  return (
    <div
      className="glass-ball absolute select-none pointer-events-none"
      data-selected={isSelected || undefined}
      data-dragging={isDragging || undefined}
      style={
        {
          // Physics owns this transform: translate (centering) + entry/pop scale.
          // No shine layers, no rim, no wobble — a plain sphere that only moves
          // when the user throws it.
          left: x,
          top: y,
          width: size,
          height: size,
          // Exposed to .glass-ball__label (font-size = radius × 0.12)
          '--ball-radius': `${radius}px`,
          transform: `translate3d(-50%, -50%, 0) scale(${scale})`,
          opacity: popping ? opacity : 1,
          transition: popping ? 'none' : 'opacity 0.3s ease',
          zIndex: isDragging ? 50 : isSelected ? 40 : 10,
        } as React.CSSProperties
      }
    >
      {/* Hover / selected / drag scale (CSS-driven, never inline) */}
      <div className="glass-ball__lift">
        {/* Plain frosted body: gradient + backdrop blur, clipped to the sphere */}
        <div className="glass-ball__body">
          {/* Ambient-occlusion contact pool — behind the model */}
          <div className="glass-ball__ao" />

          {/* Suspended 3D device — class centers it (inset 0 + flex) and lifts 3% */}
          <div
            className="glass-ball__device pointer-events-auto"
            style={{ cursor: isDragging ? 'grabbing' : 'grab' }}
          >
            <DeviceModel s={modelSize} />
          </div>

          {/* Device name — font/truncate handled by the class via --ball-radius */}
          <div className="glass-ball__label">{name}</div>
        </div>
      </div>
    </div>
  );
};

export default DeviceBubble;
