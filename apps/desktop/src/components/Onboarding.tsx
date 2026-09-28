import React, { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Check, ChevronRight, Smartphone, Monitor, Shield, Zap } from 'lucide-react';
import { cn } from '../lib/utils';

interface OnboardingProps {
  onComplete: () => void;
}

// Ink-safe icon colors for the white frosted card (spec §B5 / §B7):
// bright cyan (#00F0FF) and violet fail contrast on white — the per-step hues
// become --accent-ink + the ink semantic pairs instead.
// Every line below is a claim the user can check against the build, so each one
// is pinned to what the code actually does:
//
//  * Devices are found over mDNS and talk directly on the LAN. The cloud relay
//    client is compiled out (`spawn_relay_client` is commented out in main.rs),
//    so nothing is sent to a server — but Conduit is not "pure peer-to-peer" as
//    a privacy guarantee, because the traffic itself is unprotected.
//  * No message type is end-to-end encrypted: only call control uses the
//    envelope, and the relay terminates it anyway. See MESSAGE_PROTECTION in
//    useEncryption.ts, which is also what the status bar chip reports.
//  * Notifications, clipboard history, file transfers and message threads are
//    written to a local SQLite database, so "stored on this device" is accurate.
//  * Pairing is a QR/token exchange with no account system, so "no account is
//    required" is accurate. See PairingFlow and the Ctrl+N handler in App.tsx.
const steps = [
  {
    title: 'Welcome to Conduit',
    subtitle: 'Continuity between your desktop and your phone.',
    content: 'Conduit relays clipboard content, files, messages, notifications, and calls between devices on the same local network.',
    icon: Zap,
    iconColor: 'text-accent-ink',
  },
  {
    title: 'Connect a device',
    subtitle: 'Pair a phone with this desktop.',
    content: 'Scan the QR code shown in the app with the Conduit mobile app, or type the pairing code manually. No account is required.',
    icon: Smartphone,
    iconColor: 'text-success-ink',
  },
  {
    title: 'Where your data goes',
    subtitle: 'Kept on this device, and sent in the clear.',
    content: 'Notifications, clipboard history, and file transfers are stored in a local database on this device. Traffic between devices is not end-to-end encrypted in this build, so content sent over the network can be read by anyone on it. The cloud relay is disabled, so nothing is sent to a server.',
    icon: Shield,
    iconColor: 'text-info-ink',
  },
  {
    title: 'What you can sync',
    subtitle: 'Clipboard, files, messages, notifications, and calls.',
    content: 'Copy on your phone and paste on your desktop, send files in either direction, and relay calls and notifications to the other device.',
    icon: Monitor,
    iconColor: 'text-warning-ink',
  },
  {
    title: 'Ready to pair',
    subtitle: 'Pair a device to begin.',
    content: 'Use the Pair button in the dock, or press Ctrl+N, to start pairing.',
    icon: Check,
    iconColor: 'text-success-ink',
  },
];

const Onboarding: React.FC<OnboardingProps> = ({ onComplete }) => {
  const [step, setStep] = useState(0);

  const next = () => {
    if (step < steps.length - 1) {
      setStep(step + 1);
    } else {
      onComplete();
    }
  };

  const skip = () => {
    onComplete();
  };

  const StepIcon = steps[step].icon;

  return (
    <div
      className="fixed inset-0 z-[999] flex items-center justify-center"
      style={{
        background: 'rgba(0, 0, 0, 0.55)',
        backdropFilter: 'blur(10px)',
        WebkitBackdropFilter: 'blur(10px)',
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Conduit onboarding"
        className="surf-frost w-[460px] rounded-[24px] overflow-hidden"
      >
        {/* Body — 32 / 32 / 24 keeps every child well clear of the card edges */}
        <AnimatePresence mode="wait">
          <motion.div
            key={step}
            initial={{ opacity: 0, x: 50 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -50 }}
            transition={{ duration: 0.3, ease: [0.16, 1, 0.3, 1] }}
            className="px-8 pt-8 pb-6 text-center"
          >
            {/* Icon tile */}
            <div
              className="w-14 h-14 rounded-2xl flex items-center justify-center mx-auto mb-4 bg-accent-dim-ink"
            >
              <StepIcon size={26} strokeWidth={1.5} className={steps[step].iconColor} />
            </div>

            {/* Text */}
            <h2 className="text-[18px] [font-weight:650] text-ink-1 mb-2">
              {steps[step].title}
            </h2>
            <p className="text-[13px] font-medium text-accent-ink mb-2.5">
              {steps[step].subtitle}
            </p>
            <p className="text-[13px] font-normal leading-[1.6] text-ink-2 max-w-[340px] mx-auto">
              {steps[step].content}
            </p>
          </motion.div>
        </AnimatePresence>

        {/* Footer — hairline-separated so Skip/Next never collide with the body */}
        <div
          className="px-8 pt-4 pb-5 border-t border-line-1"
        >
          {/* Dots */}
          <div className="flex items-center justify-center gap-2 mb-5">
            {steps.map((s, i) => (
              <button
                key={i}
                type="button"
                aria-label={`Step ${i + 1} of ${steps.length}: ${s.title}`}
                aria-current={i === step ? 'step' : undefined}
                className={cn(
                  'h-1.5 rounded-full transition-all duration-300',
                  i === step
                    ? 'w-6 bg-accent-ink'
                    : 'w-1.5 bg-line-2 hover:bg-ink-3'
                )}
                onClick={() => { setStep(i); }}
              />
            ))}
          </div>

          {/* Actions */}
          <div className="flex items-center justify-between gap-4">
            {step < steps.length - 1 ? (
              <button
                type="button"
                onClick={skip}
                className="h-9 px-[14px] inline-flex items-center rounded-lg text-[13px] font-medium text-ink-3 hover:text-ink-1 transition-colors"
              >
                Skip
              </button>
            ) : (
              <div />
            )}
            <button
              type="button"
              onClick={next}
              className="h-9 px-5 inline-flex items-center gap-2 rounded-full btn-solid text-[13px] font-semibold transition-all duration-200 hover:-translate-y-px"
            >
              {step === steps.length - 1 ? 'Finish' : 'Next'}
              {step < steps.length - 1 && <ChevronRight size={16} strokeWidth={1.5} />}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Onboarding;
