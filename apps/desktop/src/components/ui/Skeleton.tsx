import { cn } from '../../lib/utils';

interface SkeletonProps {
  className?: string;
  variant?: 'text' | 'circular' | 'rectangular';
}

const Skeleton: React.FC<SkeletonProps> = ({ className, variant = 'text' }) => {
  const base = 'animate-shimmer rounded-md';

  const variants = {
    text: 'h-4 w-full',
    circular: 'rounded-full',
    rectangular: 'h-20 w-full rounded-2xl',
  };

  return (
    <div
      className={cn(base, variants[variant], className)}
      style={{
        // Neutral slate shimmer reads on BOTH white frost panels and the
        // dark starfield (the old --bg-2/--bg-3 white alphas vanish on frost).
        background: 'linear-gradient(90deg, rgba(148,163,184,0.16) 25%, rgba(148,163,184,0.30) 50%, rgba(148,163,184,0.16) 75%)',
        backgroundSize: '200% 100%',
      }}
    />
  );
};

// Pre-built skeleton layouts
export const SkeletonCard: React.FC<{ className?: string }> = ({ className }) => (
  <div
    className={cn('p-4 rounded-2xl border', className)}
    style={{ borderColor: 'rgba(148, 163, 184, 0.22)', background: 'rgba(148, 163, 184, 0.12)' }}
  >
    <div className="flex items-center gap-3">
      <Skeleton variant="circular" className="h-10 w-10 shrink-0" />
      <div className="flex-1 space-y-2">
        <Skeleton className="h-3 w-1/3" />
        <Skeleton className="h-2.5 w-2/3" />
      </div>
    </div>
  </div>
);

export const SkeletonList: React.FC<{ count?: number; className?: string }> = ({ count = 4, className }) => (
  <div className={cn('space-y-2', className)}>
    {Array.from({ length: count }).map((_, i) => (
      <SkeletonCard key={i} />
    ))}
  </div>
);

export const SkeletonPage: React.FC = () => (
  <div className="p-6 space-y-6">
    <Skeleton className="h-6 w-48" />
    <SkeletonList count={3} />
  </div>
);

export default Skeleton;
