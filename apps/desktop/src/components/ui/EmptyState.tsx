import type { LucideIcon } from 'lucide-react';
import { cn } from '../../lib/utils';

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description: string;
  action?: {
    label: string;
    onClick: () => void;
  };
  className?: string;
}

const EmptyState: React.FC<EmptyStateProps> = ({ icon: Icon, title, description, action, className }) => {
  return (
    <div
      className={cn(
        'surf-frost mx-auto flex w-[360px] max-w-[90vw] flex-col items-center justify-center rounded-[20px] px-7 py-8 text-center',
        className,
      )}
      role="status"
      aria-label={`${title}. ${description}`}
    >
      <div
        className="mb-5 flex h-16 w-16 items-center justify-center rounded-[18px]"
        style={{ background: 'rgba(0, 240, 255, 0.12)' }}
      >
        <Icon size={28} strokeWidth={1.5} className="text-accent-ink" aria-hidden="true" />
      </div>
      <h3
        className="mb-1.5 text-[15px] text-ink-1"
        style={{ fontWeight: 650 }}
      >
        {title}
      </h3>
      <p className="mb-5 max-w-[280px] text-[13px] font-normal leading-[1.6] text-ink-2">
        {description}
      </p>
      {action && (
        <button
          onClick={action.onClick}
          className="btn-press flex h-10 items-center rounded-full bg-accent-ink px-5 text-[13px] font-semibold text-on-accent transition-all duration-200 hover:-translate-y-px hover:opacity-90"
        >
          {action.label}
        </button>
      )}
    </div>
  );
};

export default EmptyState;
