import { Component, ReactNode } from 'react';

interface Props {
  children: ReactNode;
  fallback?: (error: Error, reset: () => void) => ReactNode;
  onError?: (error: Error, errorInfo: any) => void;
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

  componentDidCatch(error: Error, errorInfo: any) {
    console.error('[ErrorBoundary] Caught error:', error, errorInfo);
    this.props.onError?.(error, errorInfo);
  }

  reset = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error!, this.reset);
      }

      return (
        <div style={{
          padding: '20px',
          margin: '20px',
          border: '1px solid #ff4444',
          borderRadius: '8px',
          backgroundColor: '#ffebee',
          color: '#c62828'
        }}>
          <h2 style={{ margin: '0 0 10px 0' }}>Something went wrong</h2>
          <p style={{ margin: '0 0 15px 0' }}>{this.state.error?.message}</p>
          <button
            onClick={this.reset}
            style={{
              padding: '8px 16px',
              backgroundColor: '#c62828',
              color: 'white',
              border: 'none',
              borderRadius: '4px',
              cursor: 'pointer'
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
