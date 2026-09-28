import React, { useRef, useCallback } from 'react';
import { motion, useMotionValue, useTransform, animate } from 'motion/react';
import { cn } from '../../lib/utils';

interface SwipeableCardProps {
  children: React.ReactNode;
  onSwipeLeft?: () => void;
  onSwipeRight?: () => void;
  threshold?: number;
  leftLabel?: string;
  rightLabel?: string;
  leftColor?: string;
  rightColor?: string;
  leftIcon?: React.ReactNode;
  rightIcon?: React.ReactNode;
  disabled?: boolean;
  className?: string;
}

const SwipeableCard: React.FC<SwipeableCardProps> = ({
  children,
  onSwipeLeft,
  onSwipeRight,
  threshold = 80,
  leftLabel = 'Mark Read',
  rightLabel = 'Dismiss',
  leftColor = 'rgba(16,185,129,0.15)',
  rightColor = 'rgba(248,113,113,0.15)',
  leftIcon,
  rightIcon,
  disabled = false,
  className,
}) => {
  const x = useMotionValue(0);
  const dragging = useRef(false);

  // Background reveal opacity scales with drag distance
  const leftReveal = useTransform(x, [0, threshold], [0, 1], { clamp: true });
  const rightReveal = useTransform(x, [-threshold, 0], [1, 0], { clamp: true });
  const leftBg = useTransform(x, [0, threshold], [leftColor, 'transparent'], { clamp: true });
  const rightBg = useTransform(x, [-threshold, 0], ['transparent', rightColor], { clamp: true });

  // Scale down slightly while dragging
  const scale = useTransform(x, [-200, 0, 200], [0.97, 1, 0.97]);

  const handleDragEnd = useCallback(
    (_: unknown, info: { offset: { x: number }; velocity: { x: number } }) => {
      dragging.current = false;
      const dx = info.offset.x;
      const vx = info.velocity.x;

      // Trigger if dragged past threshold OR flung fast enough
      if ((dx > threshold || vx > 500) && onSwipeRight) {
        void animate(x, 400, { duration: 0.25 }).then(() => { onSwipeRight(); });
      } else if ((dx < -threshold || vx < -500) && onSwipeLeft) {
        void animate(x, -400, { duration: 0.25 }).then(() => { onSwipeLeft(); });
      } else {
        animate(x, 0, {
          type: 'spring',
          stiffness: 500,
          damping: 30,
        });
      }
    },
    [threshold, onSwipeLeft, onSwipeRight, x],
  );

  if (disabled) {
    return <div className={className}>{children}</div>;
  }

  return (
    <div className={cn('relative overflow-hidden rounded-2xl', className)}>
      {/* Left reveal background (shown when dragging right → dismiss) */}
      <motion.div
        className="absolute inset-0 flex items-center pl-5 rounded-2xl"
        style={{ opacity: leftReveal, background: rightBg }}
      >
        <div className="flex items-center gap-2 text-danger-ink text-[13px] font-medium">
          {rightIcon}
          <span>{rightLabel}</span>
        </div>
      </motion.div>

      {/* Right reveal background (shown when dragging left → mark read) */}
      <motion.div
        className="absolute inset-0 flex items-center justify-end pr-5 rounded-2xl"
        style={{ opacity: rightReveal, background: leftBg }}
      >
        <div className="flex items-center gap-2 text-success-ink text-[13px] font-medium">
          <span>{leftLabel}</span>
          {leftIcon}
        </div>
      </motion.div>

      {/* Card content */}
      <motion.div
        style={{ x, scale }}
        drag="x"
        dragConstraints={{ left: 0, right: 0 }}
        dragElastic={0.7}
        onDragStart={() => { dragging.current = true; }}
        onDragEnd={handleDragEnd}
        className="surf-clear relative cursor-grab active:cursor-grabbing"
      >
        {children}
      </motion.div>
    </div>
  );
};

export default SwipeableCard;
