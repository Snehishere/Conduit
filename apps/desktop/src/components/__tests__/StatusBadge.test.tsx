import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import StatusBadge from '../ui/StatusBadge';

describe('StatusBadge', () => {
  it('renders with connected status', () => {
    render(<StatusBadge status="connected" />);
    expect(screen.getByText('Connected')).toBeInTheDocument();
  });

  it('renders with syncing status', () => {
    render(<StatusBadge status="syncing" />);
    expect(screen.getByText('Syncing')).toBeInTheDocument();
  });

  it('renders with offline status', () => {
    render(<StatusBadge status="offline" />);
    expect(screen.getByText('Offline')).toBeInTheDocument();
  });

  it('renders with error status', () => {
    render(<StatusBadge status="error" />);
    expect(screen.getByText('Error')).toBeInTheDocument();
  });

  it('renders with pairing status', () => {
    render(<StatusBadge status="pairing" />);
    expect(screen.getByText('Pairing')).toBeInTheDocument();
  });

  it('renders with custom label', () => {
    render(<StatusBadge status="connected" label="Custom Label" />);
    expect(screen.getByText('Custom Label')).toBeInTheDocument();
  });

  it('applies sm size class by default', () => {
    const { container } = render(<StatusBadge status="connected" />);
    expect(container.firstChild).toHaveClass('text-[10px]');
  });

  it('applies md size class when specified', () => {
    const { container } = render(<StatusBadge status="connected" size="md" />);
    expect(container.firstChild).toHaveClass('text-[11px]');
  });
});
