import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import Onboarding from '../Onboarding';

vi.mock('motion/react', () => ({
  motion: { div: (props: any) => <div {...props} /> },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}));

describe('Onboarding', () => {
  /**
   * The first step used to promise "Seamless device synchronization with privacy
   * at its core"; the privacy step promised end-to-end encryption and "your data
   * never leaves your devices". None of that is true of this build, so both steps
   * now state what actually happens. These assertions pin the honest wording.
   */
  it('renders step title and description without overclaiming', () => {
    render(<Onboarding onComplete={vi.fn()} />);
    expect(screen.getByText('Welcome to Conduit')).toBeInTheDocument();
    expect(screen.getByText(/Continuity between your desktop and your phone/)).toBeInTheDocument();
    expect(screen.queryByText(/seamless/i)).not.toBeInTheDocument();

    // Advance to the step that used to claim end-to-end encryption.
    fireEvent.click(screen.getByText('Next'));
    fireEvent.click(screen.getByText('Next'));
    expect(screen.getByText('Where your data goes')).toBeInTheDocument();
    expect(screen.getByText(/not end-to-end encrypted/)).toBeInTheDocument();
    expect(screen.queryByText(/your data never leaves/i)).not.toBeInTheDocument();
  });

  it('next button advances step', () => {
    render(<Onboarding onComplete={vi.fn()} />);
    fireEvent.click(screen.getByText('Next'));
    expect(screen.getByText('Connect a device')).toBeInTheDocument();
  });

  it('skip button calls onComplete', () => {
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);
    fireEvent.click(screen.getByText('Skip'));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('shows Finish button on last step', () => {
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    for (let i = 0; i < 4; i++) {
      fireEvent.click(screen.getByText('Next'));
    }

    expect(screen.getByText('Ready to pair')).toBeInTheDocument();
    expect(screen.getByText('Finish')).toBeInTheDocument();
  });

  it('Finish calls onComplete', () => {
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    for (let i = 0; i < 4; i++) {
      fireEvent.click(screen.getByText('Next'));
    }

    fireEvent.click(screen.getByText('Finish'));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });
});
