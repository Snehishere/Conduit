import { render } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import Skeleton, { SkeletonCard, SkeletonList, SkeletonPage } from '../ui/Skeleton';

describe('Skeleton', () => {
  it('renders with default variant', () => {
    const { container } = render(<Skeleton className="h-4 w-32" />);
    expect(container.firstChild).toHaveClass('animate-shimmer');
  });

  it('renders circular variant', () => {
    const { container } = render(<Skeleton variant="circular" className="h-10 w-10" />);
    expect(container.firstChild).toHaveClass('rounded-full');
  });

  it('renders rectangular variant', () => {
    const { container } = render(<Skeleton variant="rectangular" className="h-20 w-full" />);
    expect(container.firstChild).toHaveClass('rounded-2xl');
  });
});

describe('SkeletonCard', () => {
  it('renders card skeleton', () => {
    const { container } = render(<SkeletonCard />);
    expect(container.querySelector('.animate-shimmer')).toBeInTheDocument();
  });
});

describe('SkeletonList', () => {
  it('renders list skeleton with default count of 4', () => {
    const { container } = render(<SkeletonList />);
    const cards = container.querySelectorAll('.p-4.rounded-2xl');
    expect(cards.length).toBe(4);
  });

  it('renders list skeleton with custom count', () => {
    const { container } = render(<SkeletonList count={3} />);
    const cards = container.querySelectorAll('.p-4.rounded-2xl');
    expect(cards.length).toBe(3);
  });
});

describe('SkeletonPage', () => {
  it('renders page skeleton', () => {
    const { container } = render(<SkeletonPage />);
    expect(container.querySelector('.animate-shimmer')).toBeInTheDocument();
  });
});
