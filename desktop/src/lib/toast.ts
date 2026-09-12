import toast from 'react-hot-toast';

export const showError = (message: string) => {
  toast.error(message, {
    duration: 4000,
    style: {
      background: '#c62828',
      color: 'white',
    },
  });
};

export const showSuccess = (message: string) => {
  toast.success(message, {
    duration: 3000,
    style: {
      background: '#2e7d32',
      color: 'white',
    },
  });
};

export const showInfo = (message: string) => {
  toast(message, {
    duration: 3000,
    style: {
      background: '#1565c0',
      color: 'white',
    },
  });
};

export const showWarning = (message: string) => {
  toast(message, {
    duration: 4000,
    style: {
      background: '#f57c00',
      color: 'white',
    },
  });
};
