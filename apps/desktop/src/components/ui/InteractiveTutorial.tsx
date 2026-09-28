import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { X, ChevronRight, ChevronLeft } from 'lucide-react';

interface TutorialStep {
  target: string; // CSS selector
  title: string;
  description: string;
  position?: 'top' | 'bottom' | 'left' | 'right';
}

/**
 * Unreachable today: `showTutorial` in useAppShell is initialised `false` and
 * nothing calls `setShowTutorial(true)` — App.tsx only ever passes it through to
 * clear the flag. The copy below is kept truthful so that wiring this up later
 * does not resurrect the old descriptions.
 */
const DEFAULT_STEPS: TutorialStep[] = [
  {
    target: '[data-tutorial="dock"]',
    title: 'Navigation dock',
    // Not "hover to expand secondary actions": the dock was rebuilt as a single
    // non-expanding row (see the PRIMARY_NAV/SECONDARY_NAV comment above it), so
    // there is no flyout tray and nothing to expand.
    description: 'Every screen is a button in this dock. There is no secondary tray to expand.',
    position: 'top',
  },
  {
    target: '[data-tutorial="status-bar"]',
    title: 'Status bar',
    description: 'Device count, connection state, and the current encryption state. The encryption chip reports what the traffic protection actually is — it does not claim end-to-end encryption.',
    position: 'top',
  },
  {
    target: '[data-tutorial="pair-button"]',
    title: 'Pair a device',
    description: 'Select this, or press Ctrl+N, to pair a new device using a QR code.',
    position: 'bottom',
  },
];

interface InteractiveTutorialProps {
  onComplete: () => void;
  steps?: TutorialStep[];
}

const InteractiveTutorial: React.FC<InteractiveTutorialProps> = ({
  onComplete,
  steps = DEFAULT_STEPS,
}) => {
  const [currentStep, setCurrentStep] = useState(0);
  const [targetRect, setTargetRect] = useState<DOMRect | null>(null);
  const overlayRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Assertion widens the index access — OOB resolves to undefined at runtime
    // (no `noUncheckedIndexedAccess` in this tsconfig, so a plain annotation
    // gets re-narrowed back to always-defined by control-flow analysis).
    const step = steps[currentStep] as TutorialStep | undefined;
    if (!step) { onComplete(); return; }

    const find = () => {
      const el = document.querySelector(step.target);
      if (el) {
        setTargetRect(el.getBoundingClientRect());
      } else {
        // Retry if element not found
        setTimeout(find, 200);
      }
    };
    find();
  }, [currentStep, steps, onComplete]);

  // See above: assertion so OOB reads can be undefined (CFA would otherwise
  // narrow a plain annotation back to always-defined).
  const step = steps[currentStep] as TutorialStep | undefined;
  if (!step || !targetRect) return null;

  const getPosition = () => {
    const gap = 12;
    switch (step.position || 'bottom') {
      case 'top':
        return { left: targetRect.left + targetRect.width / 2, top: targetRect.top - gap, transform: 'translate(-50%, -100%)' };
      case 'bottom':
        return { left: targetRect.left + targetRect.width / 2, top: targetRect.bottom + gap, transform: 'translate(-50%, 0)' };
      case 'left':
        return { left: targetRect.left - gap, top: targetRect.top + targetRect.height / 2, transform: 'translate(-100%, -50%)' };
      case 'right':
        return { left: targetRect.right + gap, top: targetRect.top + targetRect.height / 2, transform: 'translate(0, -50%)' };
    }
  };

  return (
    <div className="fixed inset-0 z-[400]" ref={overlayRef}>
      {/* Backdrop with cutout */}
      <div className="scrim absolute inset-0" />

      {/* Highlight ring */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        className="absolute"
        style={{
          left: targetRect.left - 4,
          top: targetRect.top - 4,
          width: targetRect.width + 8,
          height: targetRect.height + 8,
          borderRadius: 12,
          border: '2px solid var(--accent-ink)',
          boxShadow: '0 0 16px rgba(14, 116, 144, 0.45)',
          pointerEvents: 'none',
        }}
      />

      {/* Tooltip */}
      <AnimatePresence mode="wait">
        <motion.div
          key={currentStep}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -8 }}
          className="surf-frost absolute z-10 rounded-[24px]"
          style={{
            ...getPosition(),
            width: 280,
            boxShadow:
              '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
            padding: '16px',
          }}
        >
          <div className="flex items-start justify-between mb-2">
            <h3 className="text-[13px] font-semibold text-ink-1">
              {step.title}
            </h3>
            <button
              onClick={onComplete}
              aria-label="Skip tutorial"
              className="btn-press -m-1 p-0.5 rounded text-ink-3 transition-colors hover:text-ink-1"
            >
              <X size={12} />
            </button>
          </div>
          <p className="text-[12px] leading-relaxed mb-3 text-ink-2">
            {step.description}
          </p>
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-ink-3">
              {currentStep + 1} of {steps.length}
            </span>
            <div className="flex items-center gap-1.5">
              {currentStep > 0 && (
                <button
                  onClick={() => { setCurrentStep((s) => s - 1); }}
                  aria-label="Previous step"
                  className="surf-clear btn-press flex h-6 items-center rounded-full px-2.5 text-[11px] font-medium text-ink-2 transition-colors hover:bg-fill-2 hover:text-ink-1"
                >
                  <ChevronLeft size={12} />
                </button>
              )}
              <button
                onClick={() => {
                  if (currentStep < steps.length - 1) setCurrentStep((s) => s + 1);
                  else onComplete();
                }}
                className="btn-press flex h-6 items-center rounded-full btn-solid px-3 text-[11px] font-medium transition-colors"
              >
                {currentStep < steps.length - 1 ? (
                  <span className="flex items-center gap-1">Next <ChevronRight size={12} /></span>
                ) : (
                  'Finish'
                )}
              </button>
            </div>
          </div>
        </motion.div>
      </AnimatePresence>
    </div>
  );
};

export default InteractiveTutorial;
