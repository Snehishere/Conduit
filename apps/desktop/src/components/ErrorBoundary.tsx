import { Component, type ErrorInfo, type ReactNode } from 'react';

interface Props {
  children: ReactNode;
  fallback?: (error: Error, reset: () => void) => ReactNode;
  onError?: (error: Error, errorInfo: ErrorInfo) => void;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error('[ErrorBoundary] Caught error:', error, errorInfo);
    this.props.onError?.(error, errorInfo);
  }

  reset = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        // getDerivedStateFromError always populates `error`; the fallback below
        // covers the theoretical null case without a non-null assertion.
        return this.props.fallback(this.state.error ?? new Error('Unknown render error'), this.reset);
      }

      return (
        <div role="alert" style={{
          padding: '20px',
          margin: '20px',
          border: '1px solid var(--danger)',
          borderRadius: '8px',
          backgroundColor: 'var(--bg-2)',
          color: 'var(--text-1)'
        }}>
          <h2 style={{ margin: '0 0 10px 0', color: 'var(--danger)' }}>Something went wrong</h2>
          <p style={{ margin: '0 0 15px 0', color: 'var(--text-2)', fontSize: '13px' }}>
            The interface hit an unexpected error. Restart Conduit if it keeps happening.
          </p>
          <button
            onClick={this.reset}
            style={{
              padding: '8px 16px',
              backgroundColor: 'var(--danger)',
              color: 'var(--bg-0)',
              border: 'none',
              borderRadius: '6px',
              cursor: 'pointer',
              fontSize: '13px',
              fontWeight: 500,
            }}
          >
            Try again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
