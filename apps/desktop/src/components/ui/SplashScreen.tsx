import { motion } from 'motion/react';
import { useEffect } from 'react';
import ConduitLogo from '../icons/ConduitLogo';

interface SplashScreenProps {
  onComplete?: () => void;
}

const SplashScreen: React.FC<SplashScreenProps> = ({ onComplete }) => {
  useEffect(() => {
    const timer = setTimeout(() => onComplete?.(), 2400);
    return () => { clearTimeout(timer); };
  }, [onComplete]);

  return (
    <div
      className="fixed inset-0 z-[500] flex items-center justify-center overflow-hidden"
      style={{ background: 'var(--bg-0)' }}
    >

      {/* Ambient glow orbs */}
      <div style={{
        position: 'absolute',
        width: 400,
        height: 400,
        borderRadius: '50%',
        background: 'radial-gradient(circle, rgba(0,240,255,0.08) 0%, transparent 70%)',
        top: '50%',
        left: '50%',
        transform: 'translate(-50%, -50%)',
        pointerEvents: 'none',
      }} />
      <div style={{
        position: 'absolute',
        width: 300,
        height: 300,
        borderRadius: '50%',
        background: 'radial-gradient(circle, rgba(6,182,212,0.05) 0%, transparent 70%)',
        top: '40%',
        left: '55%',
        transform: 'translate(-50%, -50%)',
        pointerEvents: 'none',
      }} />

      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.3 }}
        className="flex flex-col items-center gap-5"
      >
        {/* Logo with dramatic entrance */}
        <motion.div
          initial={{ opacity: 0, scale: 0.6, rotate: -12, filter: 'blur(12px)' }}
          animate={{ opacity: 1, scale: 1, rotate: 0, filter: 'blur(0px)' }}
          transition={{ duration: 0.65, ease: [0.34, 1.56, 0.64, 1] }}
          style={{
            borderRadius: 24,
            overflow: 'hidden',
            boxShadow: '0 0 40px rgba(0,240,255,0.2), 0 0 100px rgba(0,240,255,0.08)',
          }}
        >
          <ConduitLogo size={80} withBackground />
        </motion.div>

        {/* App name with gradient */}
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.35, duration: 0.4, ease: [0.4, 0, 0.2, 1] }}
          className="flex flex-col items-center gap-1"
        >
          <span
            style={{
              fontSize: 22,
              fontWeight: 700,
              fontFamily: 'var(--font-display)',
              letterSpacing: '-0.02em',
              background: 'var(--gradient-brand)',
              WebkitBackgroundClip: 'text',
              WebkitTextFillColor: 'transparent',
              backgroundClip: 'text',
            }}
          >
            Conduit
          </span>
          <motion.span
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.55, duration: 0.3 }}
            style={{ fontSize: 11, color: 'var(--text-2)', letterSpacing: '0.04em' }}
          >
            Your devices, connected
          </motion.span>
        </motion.div>

        {/* Loading bar with gradient */}
        <motion.div
          className="relative overflow-hidden rounded-full mt-1"
          style={{ width: 140, height: 2, background: 'var(--bg-3)' }}
        >
          <motion.div
            initial={{ x: '-100%' }}
            animate={{ x: '0%' }}
            transition={{ delay: 0.5, duration: 1.6, ease: [0.4, 0, 0.2, 1] }}
            className="absolute inset-0 rounded-full"
            style={{ background: 'var(--gradient-brand)' }}
          />
        </motion.div>
      </motion.div>
    </div>
  );
};

export default SplashScreen;
