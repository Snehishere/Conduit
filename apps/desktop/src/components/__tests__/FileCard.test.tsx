import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import FileCard from '../files/FileCard';

vi.mock('framer-motion', () => ({
  motion: {
    div: (props: any) => <div {...props} />,
  },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}));

const makeTransfer = (overrides: Record<string, any> = {}) => ({
  id: '1',
  name: 'photo.png',
  size: 1024000,
  mime: 'image/png',
  from_device: 'phone',
  to_device: 'desktop',
  status: 'complete',
  chunks_received: 10,
  total_chunks: 10,
  saved_path: '/tmp/photo.png',
  timestamp: Math.floor(Date.now() / 1000) - 60,
  ...overrides,
});

const defaultProps = {
  activeTransferId: null,
  activeProgress: 0,
  onAccept: vi.fn(),
  onCancel: vi.fn(),
  onResume: vi.fn(),
  onOpen: vi.fn(),
  formatFileSize: (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`,
  getFileIcon: () => 'FileText',
};

describe('FileCard', () => {
  it('renders file name and size', () => {
    render(<FileCard transfer={makeTransfer()} {...defaultProps} />);
    expect(screen.getByText('photo.png')).toBeInTheDocument();
    expect(screen.getByText('1.0 MB')).toBeInTheDocument();
  });

  it('renders status text', () => {
    render(<FileCard transfer={makeTransfer({ status: 'complete' })} {...defaultProps} />);
    expect(screen.getByText('complete')).toBeInTheDocument();
  });

  it('shows progress bar for active transfers', () => {
    const { container } = render(
      <FileCard
        transfer={makeTransfer({ status: 'transferring', chunks_received: 5, total_chunks: 10 })}
        {...defaultProps}
        activeTransferId="1"
        activeProgress={50}
      />
    );
    const progressBar = container.querySelector('.bg-accent');
    expect(progressBar).toBeInTheDocument();
  });

  it('renders different file type icons for images', () => {
    const { container } = render(
      <FileCard transfer={makeTransfer({ mime: 'image/jpeg' })} {...defaultProps} />
    );
    expect(container.querySelector('.text-violet-400')).toBeInTheDocument();
  });

  it('renders different file type icons for audio', () => {
    const { container } = render(
      <FileCard transfer={makeTransfer({ mime: 'audio/mp3' })} {...defaultProps} />
    );
    expect(container.querySelector('.text-amber-400')).toBeInTheDocument();
  });

  it('renders different file type icons for video', () => {
    const { container } = render(
      <FileCard transfer={makeTransfer({ mime: 'video/mp4' })} {...defaultProps} />
    );
    expect(container.querySelector('.text-rose-400')).toBeInTheDocument();
  });

  it('renders different file type icons for archive', () => {
    const { container } = render(
      <FileCard transfer={makeTransfer({ mime: 'application/zip' })} {...defaultProps} />
    );
    expect(container.querySelector('.text-blue-400')).toBeInTheDocument();
  });

  it('calls onOpen when clicking a completed file', () => {
    const onOpen = vi.fn();
    render(
      <FileCard
        transfer={makeTransfer({ status: 'complete', saved_path: '/tmp/photo.png' })}
        {...defaultProps}
        onOpen={onOpen}
      />
    );
    fireEvent.click(screen.getByText('photo.png'));
    expect(onOpen).toHaveBeenCalledWith('/tmp/photo.png');
  });
});
