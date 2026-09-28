import { toast } from 'sonner';

/** Announce a message to screen readers via the live region */
function announceToScreenReader(message: string) {
  const el = document.getElementById('sr-announcer');
  if (el) {
    el.textContent = '';
    // Small delay ensures the screen reader picks up the change
    setTimeout(() => { el.textContent = message; }, 50);
  }
}

export const showError = (message: string) => {
  toast.error(message, {
    duration: 4000,
    style: {
      background: 'var(--danger)',
      color: 'var(--bg-0)',
    },
  });
  announceToScreenReader(message);
};

export const showSuccess = (message: string) => {
  toast.success(message, {
    duration: 3000,
    style: {
      background: 'var(--success)',
      color: 'var(--bg-0)',
    },
  });
  announceToScreenReader(message);
};

export const showInfo = (message: string) => {
  toast(message, {
    duration: 3000,
    style: {
      background: 'var(--info)',
      color: 'var(--bg-0)',
    },
  });
  announceToScreenReader(message);
};

export const showWarning = (message: string) => {
  toast(message, {
    duration: 4000,
    style: {
      background: 'var(--warning)',
      color: 'var(--bg-0)',
    },
  });
  announceToScreenReader(message);
};
