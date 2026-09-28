import React from 'react';
import { motion } from 'motion/react';

interface BubbleParticlesProps {
  x: number;
  y: number;
  color: string;
  radius: number;
}

const PARTICLE_COUNT = 8;

const BubbleParticles: React.FC<BubbleParticlesProps> = ({ x, y, color, radius }) => {
  const particles = React.useMemo(() => {
    return Array.from({ length: PARTICLE_COUNT }, (_, i) => {
      const angle = (i / PARTICLE_COUNT) * Math.PI * 2 + (Math.random() - 0.5) * 0.5;
      const distance = radius * (1.5 + Math.random() * 1.5);
      const size = 4 + Math.random() * 8;
      return {
        id: i,
        endX: Math.cos(angle) * distance,
        endY: Math.sin(angle) * distance,
        size,
        delay: Math.random() * 0.1,
      };
    });
  }, [radius]);

  return (
    <div
      className="absolute pointer-events-none"
      style={{ left: x, top: y, transform: 'translate(-50%, -50%)' }}
    >
      {particles.map((p) => (
        <motion.div
          key={p.id}
          className="absolute rounded-full"
          style={{
            width: p.size,
            height: p.size,
            background: color,
            left: -p.size / 2,
            top: -p.size / 2,
            filter: `blur(1px)`,
          }}
          initial={{ x: 0, y: 0, opacity: 0.8, scale: 1 }}
          animate={{
            x: p.endX,
            y: p.endY,
            opacity: 0,
            scale: 0.3,
          }}
          transition={{
            duration: 0.45,
            delay: p.delay,
            ease: 'easeOut',
          }}
        />
      ))}
      {/* Central flash */}
      <motion.div
        className="absolute rounded-full"
        style={{
          width: radius * 1.5,
          height: radius * 1.5,
          left: -radius * 0.75,
          top: -radius * 0.75,
          background: `radial-gradient(circle, ${color}40, transparent 70%)`,
        }}
        initial={{ opacity: 0.6, scale: 0.5 }}
        animate={{ opacity: 0, scale: 2 }}
        transition={{ duration: 0.35, ease: 'easeOut' }}
      />
    </div>
  );
};

export default BubbleParticles;
