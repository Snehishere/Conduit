import { describe, it, expect, vi, beforeEach } from 'vitest';
import { showError, showSuccess, showInfo, showWarning } from '../toast';

const mockToastFn = vi.hoisted(() =>
  Object.assign(vi.fn(), {
    error: vi.fn(),
    success: vi.fn(),
  })
);

vi.mock('sonner', () => ({
  toast: mockToastFn,
}));

describe('toast helpers', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('showError calls toast.error', () => {
    showError('Something went wrong');
    expect(mockToastFn.error).toHaveBeenCalledWith(
      'Something went wrong',
      expect.objectContaining({ duration: 4000 })
    );
  });

  it('showSuccess calls toast.success', () => {
    showSuccess('File sent');
    expect(mockToastFn.success).toHaveBeenCalledWith(
      'File sent',
      expect.objectContaining({ duration: 3000 })
    );
  });

  it('showInfo calls toast', () => {
    showInfo('Syncing...');
    expect(mockToastFn).toHaveBeenCalledWith(
      'Syncing...',
      expect.objectContaining({ duration: 3000 })
    );
  });

  it('showWarning calls toast', () => {
    showWarning('Low battery');
    expect(mockToastFn).toHaveBeenCalledWith(
      'Low battery',
      expect.objectContaining({ duration: 4000 })
    );
  });
});
