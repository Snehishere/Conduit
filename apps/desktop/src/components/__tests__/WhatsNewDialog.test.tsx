import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import WhatsNewDialog from '../ui/WhatsNewDialog';

vi.mock('framer-motion', () => ({
  motion: { div: (props: any) => <div {...props} /> },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}));

describe('WhatsNewDialog', () => {
  const defaultProps = {
    version: '1.2.0',
    features: ['New clipboard sync', 'Bug fixes', 'Performance improvements'],
    onComplete: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders version and feature list', () => {
    render(<WhatsNewDialog {...defaultProps} />);
    expect(screen.getByText('Version 1.2.0')).toBeInTheDocument();
    expect(screen.getByText('New clipboard sync')).toBeInTheDocument();
    expect(screen.getByText('Bug fixes')).toBeInTheDocument();
    expect(screen.getByText('Performance improvements')).toBeInTheDocument();
  });

  it('Got it button calls onComplete', () => {
    const onComplete = vi.fn();
    render(<WhatsNewDialog {...defaultProps} onComplete={onComplete} />);
    fireEvent.click(screen.getByText('Got it'));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('X button closes and calls onComplete', () => {
    const onComplete = vi.fn();
    render(<WhatsNewDialog {...defaultProps} onComplete={onComplete} />);

    const buttons = screen.getAllByRole('button');
    const xButton = buttons.find((btn) => btn.querySelector('svg'));
    expect(xButton).toBeDefined();
    fireEvent.click(xButton!);
    expect(onComplete).toHaveBeenCalledTimes(1);
  });
});
