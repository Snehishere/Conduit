import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import ShortcutsOverlay from '../ui/ShortcutsOverlay';

describe('ShortcutsOverlay', () => {
  it('renders the overlay with title when open', () => {
    render(<ShortcutsOverlay open={true} onClose={vi.fn()} />);
    expect(screen.getByText('Keyboard Shortcuts')).toBeInTheDocument();
  });

  it('renders nothing when closed', () => {
    const { container } = render(<ShortcutsOverlay open={false} onClose={vi.fn()} />);
    expect(container.innerHTML).toBe('');
  });

  it('displays navigation group', () => {
    render(<ShortcutsOverlay open={true} onClose={vi.fn()} />);
    expect(screen.getByText('Navigation')).toBeInTheDocument();
  });

  it('displays general group', () => {
    render(<ShortcutsOverlay open={true} onClose={vi.fn()} />);
    expect(screen.getByText('General')).toBeInTheDocument();
  });

  it('calls onClose when close button is clicked', () => {
    const onClose = vi.fn();
    render(<ShortcutsOverlay open={true} onClose={onClose} />);
    const closeBtn = screen.getByRole('button');
    fireEvent.click(closeBtn);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when clicking backdrop', () => {
    const onClose = vi.fn();
    const { container } = render(<ShortcutsOverlay open={true} onClose={onClose} />);
    fireEvent.click(container.firstChild!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
