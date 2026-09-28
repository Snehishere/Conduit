import React, { useEffect, useState, useCallback, useRef } from 'react';
import { Call } from '../../hooks/useCalls';
import {
  Phone, PhoneOff, PhoneIncoming, PhoneCall, ArrowLeft, ArrowRight, Forward,
} from 'lucide-react';
import { motion, AnimatePresence, useMotionValue, useTransform, useSpring, animate } from 'motion/react';

interface Device {
  id: string;
  name: string;
  status: string;
}

interface IncomingCallProps {
  call: Call;
  onAnswer: () => void;
  onReject: () => void;
  onForward: (callId: string, toDeviceId: string) => void;
  getCallerName: (call: Call) => string;
  getCallDuration: (call: Call) => string;
  devices: Device[];
  embedded?: boolean;
}

/**
 * Call window, centred on screen when floating and inline in the calls panel
 * when `embedded`. The card itself is draggable: drag right to accept, left to
 * decline. A two-finger trackpad swipe does the same thing (see
 * `handleTrackpadWheel`), and Shift+Left / Shift+Right are the keyboard
 * equivalents.
 *
 * Surface: .surf-glass obsidian HUD (TEXT tokens, spec §B4) — when `embedded`
 * the surrounding swipe-indicator overlays sit on the white frost panel, so
 * those two overlays switch to the ink semantic colors (spec §B7).
 */
const IncomingCall: React.FC<IncomingCallProps> = ({
  call,
  onAnswer,
  onReject,
  onForward,
  getCallerName,
  getCallDuration,
  devices,
  embedded = false,
}) => {
  const [duration, setDuration] = useState('0:00');
  const [isHoveringPickup, setIsHoveringPickup] = useState(false);
  const [isHoveringHangup, setIsHoveringHangup] = useState(false);
  const [showForwardMenu, setShowForwardMenu] = useState(false);
  const trackpadDeltaRef = useRef(0);
  const trackpadTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const connectedDevices = devices.filter((d) => d.status === 'connected');

  // Motion values for swipe gestures
  const x = useMotionValue(0);
  const cardRotate = useTransform(x, [-200, 0, 200], [-5, 0, 5]);
  const pickupScale = useTransform(x, [0, 150], [1, 1.15], { clamp: true });
  const hangupScale = useTransform(x, [-150, 0], [1.15, 1], { clamp: true });
  const bgGreen = useTransform(x, [0, 120], [0, 0.15], { clamp: true });
  const bgRed = useTransform(x, [-120, 0], [0.15, 0], { clamp: true });

  // Spring physics for smooth interactions
  const springConfig = { stiffness: 500, damping: 30 };
  const pickupSpring = useSpring(pickupScale, springConfig);
  const hangupSpring = useSpring(hangupScale, springConfig);

  // Trackpads report two-finger horizontal swipes as horizontal wheel deltas.
  // Left rejects and right accepts, matching the on-screen gesture hints.
  const handleTrackpadWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    if (call.status !== 'ringing' || Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault();
    trackpadDeltaRef.current += event.deltaX;
    if (trackpadTimerRef.current) clearTimeout(trackpadTimerRef.current);
    trackpadTimerRef.current = setTimeout(() => {
      const delta = trackpadDeltaRef.current;
      trackpadDeltaRef.current = 0;
      trackpadTimerRef.current = null;
      if (delta <= -80) onReject();
      else if (delta >= 80) onAnswer();
    }, 100);
  }, [call.status, onAnswer, onReject]);

  useEffect(() => () => {
    if (trackpadTimerRef.current) clearTimeout(trackpadTimerRef.current);
  }, []);

  // Update duration timer for active calls
  useEffect(() => {
    if (call.status !== 'active') return;

    const interval = setInterval(() => {
      setDuration(getCallDuration(call));
    }, 1000);

    return () => { clearInterval(interval); };
  }, [call.status, call, getCallDuration]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.shiftKey && e.key === 'ArrowLeft') {
        e.preventDefault();
        onReject();
      } else if (e.shiftKey && e.key === 'ArrowRight') {
        e.preventDefault();
        onAnswer();
      }
    };

    window.addEventListener('keydown', handler);
    return () => { window.removeEventListener('keydown', handler); };
  }, [onAnswer, onReject]);

  const handleDragEnd = useCallback((_: unknown, info: { offset: { x: number }; velocity: { x: number } }) => {
    const dx = info.offset.x;
    const vx = info.velocity.x;

    if (dx > 120 || vx > 800) {
      void animate(x, 600, { duration: 0.3, ease: [0.32, 0.72, 0, 1] }).then(() => { onAnswer(); });
    } else if (dx < -120 || vx < -800) {
      void animate(x, -600, { duration: 0.3, ease: [0.32, 0.72, 0, 1] }).then(() => { onReject(); });
    } else {
      animate(x, 0, { type: 'spring', stiffness: 400, damping: 25 });
    }
  }, [x, onAnswer, onReject]);

  const getStatusInfo = () => {
    switch (call.status) {
      case 'ringing':
        return {
          icon: <PhoneIncoming size={14} className="text-accent" />,
          text: 'Incoming call',
          color: 'text-accent',
          pulse: true,
        };
      case 'active':
        return {
          icon: <PhoneCall size={14} className="text-success" />,
          text: duration,
          color: 'text-success',
          pulse: false,
        };
      default:
        return {
          icon: <Phone size={14} className="text-text-secondary" />,
          text: '',
          color: 'text-text-secondary',
          pulse: false,
        };
    }
  };

  const statusInfo = getStatusInfo();

  // Swipe-indicator overlays: ink tokens on white frost (embedded), text
  // tokens on the dimmed starfield/scrim (floating window).
  const acceptTone = embedded ? 'text-success-ink' : 'text-success';
  const acceptFill = embedded ? 'bg-success-ink/20' : 'bg-success/20';
  const declineTone = embedded ? 'text-danger-ink' : 'text-danger';
  const declineFill = embedded ? 'bg-danger-ink/20' : 'bg-danger/20';

  return (
    <AnimatePresence>
      <motion.div
        className={embedded
          ? 'relative h-full min-h-[460px] w-full flex items-center justify-center'
          : 'fixed inset-0 z-[200] flex items-center justify-center'}
        onWheel={handleTrackpadWheel}
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
      >
        {!embedded && (
          <motion.div
            className="absolute inset-0 bg-black/50 backdrop-blur-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          />
        )}

        {/* Swipe indicator backgrounds */}
        <motion.div
          className="absolute inset-0 flex items-center justify-start"
          style={{ opacity: bgGreen, paddingLeft: '22%' }}
        >
          <motion.div
            className={`flex items-center gap-3 text-[15px] font-semibold ${acceptTone}`}
            style={{ scale: pickupSpring }}
          >
            <div className={`w-14 h-14 rounded-full flex items-center justify-center ${acceptFill}`}>
              <Phone size={24} />
            </div>
            <span>Accept</span>
          </motion.div>
        </motion.div>
        <motion.div
          className="absolute inset-0 flex items-center justify-end"
          style={{ opacity: bgRed, paddingRight: '22%' }}
        >
          <motion.div
            className={`flex items-center gap-3 text-[15px] font-semibold ${declineTone}`}
            style={{ scale: hangupSpring }}
          >
            <span>Decline</span>
            <div className={`w-14 h-14 rounded-full flex items-center justify-center ${declineFill}`}>
              <PhoneOff size={24} />
            </div>
          </motion.div>
        </motion.div>

        {/* Main call window */}
        <motion.div
          className={`relative ${embedded ? 'w-full max-w-[440px] max-h-full overflow-y-auto' : 'w-[440px]'} surf-glass rounded-[24px] shadow-2xl overflow-hidden cursor-grab active:cursor-grabbing`}
          style={{ x, rotate: cardRotate }}
          drag="x"
          dragConstraints={{ left: 0, right: 0 }}
          dragElastic={0.7}
          onDragEnd={handleDragEnd}
          initial={{ scale: 0.85, opacity: 0, y: 30 }}
          animate={{ scale: 1, opacity: 1, y: 0 }}
          exit={{ scale: 0.85, opacity: 0, y: 30 }}
          transition={{ type: 'spring', stiffness: 500, damping: 35 }}
        >
          {/* Window title bar */}
          <div className="flex items-center justify-between bg-bg-2 border-b border-border" style={{ padding: '10px 16px' }}>
            <div className="flex items-center gap-2">
              <div className="flex gap-1.5">
                <div className="w-3 h-3 rounded-full bg-danger/80" />
                <div className="w-3 h-3 rounded-full bg-warning/80" />
                <div className="w-3 h-3 rounded-full bg-success/80" />
              </div>
            </div>
            <span className="text-[12px] text-text-secondary font-medium">Call</span>
            <div className="w-[52px]" /> {/* Spacer for centering */}
          </div>

          {/* Content area */}
          <div className="text-center" style={{ padding: '32px 32px 24px' }}>
            {/* Avatar with animated ring */}
            <div className="relative flex justify-center" style={{ width: 'fit-content', margin: '0 auto 24px' }}>
              <motion.div
                className="absolute -inset-3 rounded-full border-2 border-accent/20"
                animate={call.status === 'ringing' ? {
                  scale: [1, 1.08, 1],
                  opacity: [0.2, 0.5, 0.2],
                } : {}}
                transition={{ duration: 2, repeat: Infinity, ease: 'easeInOut' }}
              />
              <motion.div
                className="absolute -inset-5 rounded-full border border-accent/15"
                animate={call.status === 'ringing' ? {
                  scale: [1, 1.12, 1],
                  opacity: [0.15, 0.35, 0.15],
                } : {}}
                transition={{ duration: 2, repeat: Infinity, ease: 'easeInOut', delay: 0.4 }}
              />
              <motion.div
                className="w-24 h-24 rounded-full bg-accent/10 border-2 border-accent/25 flex items-center justify-center"
                whileHover={{ scale: 1.03 }}
                transition={{ type: 'spring', stiffness: 400, damping: 20 }}
              >
                <span className="text-[36px] font-bold text-accent">
                  {getCallerName(call).charAt(0).toUpperCase()}
                </span>
              </motion.div>
            </div>

            {/* Caller info */}
            <motion.h2
              className="text-[24px] font-bold text-text-primary"
              style={{ marginBottom: 6 }}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.1 }}
            >
              {getCallerName(call)}
            </motion.h2>
            <motion.p
              className="text-[16px] text-text-secondary font-mono"
              style={{ marginBottom: 20 }}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.15 }}
            >
              {call.number}
            </motion.p>

            {/* Status badge */}
            <motion.div
              className="inline-flex items-center gap-2 rounded-full bg-bg-2 border border-border"
              style={{ padding: '8px 16px' }}
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ delay: 0.2 }}
            >
              {statusInfo.icon}
              <span className={`text-[13px] font-medium ${statusInfo.color}`}>
                {statusInfo.text}
              </span>
              {statusInfo.pulse && (
                <motion.span
                  className="w-2 h-2 rounded-full bg-accent"
                  animate={{ opacity: [1, 0.3, 1] }}
                  transition={{ duration: 1.5, repeat: Infinity }}
                />
              )}
            </motion.div>
          </div>

          {/* Swipe hints */}
          <div style={{ padding: '0 32px 8px' }}>
            <div className="flex items-center justify-between text-[11px] text-text-secondary/70">
              <motion.div
                className="flex items-center gap-1"
                animate={{ x: [-2, 2, -2] }}
                transition={{ duration: 2.5, repeat: Infinity }}
              >
                <ArrowLeft size={11} />
                <span>Decline</span>
              </motion.div>
              <span className="text-[10px]">swipe or drag</span>
              <motion.div
                className="flex items-center gap-1"
                animate={{ x: [2, -2, 2] }}
                transition={{ duration: 2.5, repeat: Infinity }}
              >
                <span>Accept</span>
                <ArrowRight size={11} />
              </motion.div>
            </div>
          </div>

          {/* Action buttons */}
          <div style={{ padding: '16px 32px 32px' }}>
            <div className="flex items-center justify-center gap-10">
              {/* Reject button */}
              <motion.button
                onClick={onReject}
                onMouseEnter={() => { setIsHoveringHangup(true); }}
                onMouseLeave={() => { setIsHoveringHangup(false); }}
                className="relative group"
                whileHover={{ scale: 1.1, y: -2 }}
                whileTap={{ scale: 0.95 }}
              >
                <motion.div
                  className="absolute -inset-3 rounded-full bg-danger/15 blur-xl"
                  animate={{ opacity: isHoveringHangup ? 0.8 : 0 }}
                  transition={{ duration: 0.2 }}
                />
                <motion.div
                  className="relative w-16 h-16 rounded-full bg-danger/10 border border-danger/25 flex items-center justify-center text-danger group-hover:bg-danger/15 group-hover:border-danger/40 transition-all duration-200"
                  animate={call.status === 'ringing' ? {} : {}}
                >
                  <PhoneOff size={24} strokeWidth={1.8} />
                </motion.div>
                <p className="text-[12px] text-text-secondary text-center font-medium" style={{ marginTop: 10 }}>Decline</p>
                <p className="text-[10px] text-text-secondary/70 text-center font-mono" style={{ marginTop: 2 }}>⇧ ←</p>
              </motion.button>

              {/* Answer button */}
              <motion.button
                onClick={onAnswer}
                onMouseEnter={() => { setIsHoveringPickup(true); }}
                onMouseLeave={() => { setIsHoveringPickup(false); }}
                className="relative group"
                whileHover={{ scale: 1.1, y: -2 }}
                whileTap={{ scale: 0.95 }}
              >
                <motion.div
                  className="absolute -inset-3 rounded-full bg-success/15 blur-xl"
                  animate={{ opacity: isHoveringPickup ? 0.8 : 0 }}
                  transition={{ duration: 0.2 }}
                />
                <motion.div
                  className="relative w-16 h-16 rounded-full bg-success/10 border border-success/25 flex items-center justify-center text-success group-hover:bg-success/15 group-hover:border-success/40 transition-all duration-200"
                  animate={call.status === 'ringing' ? {
                    boxShadow: [
                      '0 0 0 0 rgba(34, 197, 94, 0)',
                      '0 0 0 10px rgba(34, 197, 94, 0.08)',
                      '0 0 0 0 rgba(34, 197, 94, 0)',
                    ],
                  } : {}}
                  transition={{ duration: 2, repeat: Infinity }}
                >
                  <Phone size={24} strokeWidth={1.8} />
                </motion.div>
                <p className="text-[12px] text-text-secondary text-center font-medium" style={{ marginTop: 10 }}>Accept</p>
                <p className="text-[10px] text-text-secondary/70 text-center font-mono" style={{ marginTop: 2 }}>⇧ →</p>
              </motion.button>
            </div>

            {/* Forward to phone button */}
            <div className="flex justify-center relative" style={{ marginTop: 16 }}>
              <motion.button
                onClick={() => { setShowForwardMenu(!showForwardMenu); }}
                className="surf-clear-on-dark flex items-center gap-2 rounded-lg text-text-secondary hover:text-text-primary hover:bg-fill-2! transition-colors text-[13px] font-medium"
                style={{ padding: '8px 16px', border: '1px solid var(--line-1)' }}
                whileHover={{ scale: 1.03 }}
                whileTap={{ scale: 0.97 }}
              >
                <Forward size={14} />
                <span>Forward to Phone</span>
              </motion.button>
              {showForwardMenu && connectedDevices.length > 0 && (
                <motion.div
                  className="absolute bottom-full left-1/2 -translate-x-1/2 bg-bg-primary border border-border rounded-lg shadow-lg overflow-hidden z-10 min-w-[180px]"
                  style={{ marginBottom: 8 }}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.15 }}
                >
                  {connectedDevices.map((device) => (
                    <button
                      key={device.id}
                      onClick={() => {
                        onForward(call.call_id, device.id);
                        setShowForwardMenu(false);
                      }}
                      className="w-full text-left text-[13px] text-text-primary hover:bg-bg-2! transition-colors flex items-center gap-2"
                      style={{ padding: '10px 16px' }}
                    >
                      <Phone size={12} className="text-accent" />
                      {device.name}
                    </button>
                  ))}
                </motion.div>
              )}
            </div>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
};

export default IncomingCall;
