import { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Sparkles, X, ChevronRight } from 'lucide-react';

interface WhatsNewDialogProps {
  version: string;
  features: string[];
  onComplete: () => void;
}

const WhatsNewDialog: React.FC<WhatsNewDialogProps> = ({ version, features, onComplete }) => {
  const [dismissed, setDismissed] = useState(false);

  if (dismissed) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="scrim fixed inset-0 z-[250] flex items-center justify-center"
        onClick={onComplete}
      >
        <motion.div
          initial={{ opacity: 0, scale: 0.95, y: 10 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.95, y: 10 }}
          transition={{ duration: 0.3, ease: [0.16, 1, 0.3, 1] }}
          className="surf-frost w-[400px] max-w-[90vw] rounded-[24px]"
          style={{
            boxShadow:
              '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
          }}
          onClick={(e) => { e.stopPropagation(); }}
        >
          {/* Header */}
          <div className="flex items-center justify-between px-5 pt-5 pb-2">
            <div className="flex items-center gap-2.5">
              <div
                className="w-8 h-8 rounded-lg flex items-center justify-center bg-accent-dim-ink"
              >
                <Sparkles size={16} className="text-accent-ink" />
              </div>
              <div>
                <h2 className="text-[15px] font-semibold text-ink-1">
                  What's New
                </h2>
                <p className="text-[11px] text-ink-3">
                  Version {version}
                </p>
              </div>
            </div>
            <button
              onClick={() => { setDismissed(true); onComplete(); }}
              className="btn-press w-7 h-7 flex items-center justify-center rounded-lg text-ink-3 transition-colors hover:bg-fill-2 hover:text-ink-1"
            >
              <X size={14} />
            </button>
          </div>

          {/* Features */}
          <div className="px-5 py-3 space-y-2">
            {features.map((feature, i) => (
              <motion.div
                key={i}
                initial={{ opacity: 0, x: -8 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: 0.1 + i * 0.05 }}
                className="flex items-start gap-2.5 py-1.5"
              >
                <ChevronRight
                  size={14}
                  className="mt-0.5 shrink-0 text-accent-ink"
                />
                <span className="text-[12px] leading-relaxed text-ink-2">
                  {feature}
                </span>
              </motion.div>
            ))}
          </div>

          {/* Footer */}
          <div className="px-5 pb-5 pt-2">
            <button
              onClick={() => { setDismissed(true); onComplete(); }}
              className="btn-press flex h-9 w-full items-center justify-center rounded-full btn-solid text-[13px] font-semibold transition-colors"
            >
              Got it
            </button>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
};

export default WhatsNewDialog;
