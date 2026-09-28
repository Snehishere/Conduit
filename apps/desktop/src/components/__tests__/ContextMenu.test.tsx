import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import ContextMenu from '../ui/ContextMenu';

describe('ContextMenu', () => {
  const trigger = <span>Right-click me</span>;

  it('renders items with labels', () => {
    render(
      <ContextMenu
        items={[
          { label: 'Copy', onClick: vi.fn() },
          { label: 'Paste', onClick: vi.fn() },
        ]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));

    expect(screen.getByText('Copy')).toBeInTheDocument();
    expect(screen.getByText('Paste')).toBeInTheDocument();
  });

  it('renders separator', () => {
    render(
      <ContextMenu
        items={[
          { label: 'Copy', onClick: vi.fn() },
          { type: 'separator' },
          { label: 'Delete', onClick: vi.fn() },
        ]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));

    expect(screen.getByText('Copy')).toBeInTheDocument();
    expect(screen.getByText('Delete')).toBeInTheDocument();
  });

  it('calls onClick when item clicked', () => {
    const onClick = vi.fn();
    render(
      <ContextMenu
        items={[{ label: 'Copy', onClick }]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));
    fireEvent.click(screen.getByText('Copy'));

    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('shows shortcut text', () => {
    render(
      <ContextMenu
        items={[{ label: 'Copy', shortcut: 'Ctrl+C', onClick: vi.fn() }]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));

    expect(screen.getByText('Ctrl+C')).toBeInTheDocument();
  });

  it('renders danger items', () => {
    render(
      <ContextMenu
        items={[{ label: 'Delete', danger: true, onClick: vi.fn() }]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));

    const deleteBtn = screen.getByText('Delete');
    expect(deleteBtn).toBeInTheDocument();
    expect(deleteBtn.closest('button')).toHaveClass('text-red-400');
  });

  it('closes on Escape', () => {
    render(
      <ContextMenu
        items={[{ label: 'Copy', onClick: vi.fn() }]}
      >
        {trigger}
      </ContextMenu>
    );

    fireEvent.contextMenu(screen.getByText('Right-click me'));
    expect(screen.getByText('Copy')).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByText('Copy')).not.toBeInTheDocument();
  });
});
