import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import EmptyState from '../ui/EmptyState';
import { Folder } from 'lucide-react';

describe('EmptyState', () => {
  it('renders title and description', () => {
    render(
      <EmptyState
        icon={Folder}
        title="No files"
        description="Upload files to get started"
      />
    );
    expect(screen.getByText('No files')).toBeInTheDocument();
    expect(screen.getByText('Upload files to get started')).toBeInTheDocument();
  });

  it('renders action button when provided', () => {
    const onClick = vi.fn();
    render(
      <EmptyState
        icon={Folder}
        title="No files"
        description="Upload files to get started"
        action={{ label: 'Upload', onClick }}
      />
    );
    const button = screen.getByText('Upload');
    expect(button).toBeInTheDocument();
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('does not render action button when not provided', () => {
    render(
      <EmptyState
        icon={Folder}
        title="No files"
        description="Upload files to get started"
      />
    );
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });
});
