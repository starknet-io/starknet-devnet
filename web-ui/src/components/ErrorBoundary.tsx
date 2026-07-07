import { Component, type ErrorInfo, type ReactNode } from 'react';
import { AlertTriangle, RefreshCw } from 'lucide-react';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

/**
 * Top-level error boundary. Catches render-time errors anywhere in the
 * route tree and shows a recoverable fallback instead of a white screen.
 */
export default class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error('[ErrorBoundary] Caught error:', error, info.componentStack);
  }

  private handleReset = (): void => {
    this.setState({ error: null });
  };

  private handleReload = (): void => {
    window.location.reload();
  };

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex flex-col items-center justify-center h-full p-8">
        <div className="w-full max-w-lg">
          <div className="text-center mb-8">
            <div className="inline-flex items-center justify-center w-16 h-16 rounded-full bg-red-500/10 border border-red-500/20 mb-4">
              <AlertTriangle size={28} className="text-red-400" />
            </div>
            <h2 className="text-xl font-semibold mb-2">Something went wrong</h2>
            <p className="text-gray-400 text-sm">
              The explorer hit an unexpected error rendering this view.
            </p>
            <pre className="mt-4 text-left text-xs text-red-300 bg-red-500/10 border border-red-500/20 rounded-lg p-3 overflow-auto max-h-48 whitespace-pre-wrap break-all">
              {error.message}
            </pre>
          </div>
          <div className="flex justify-center gap-3">
            <button onClick={this.handleReset} className="btn-secondary text-sm px-4 py-2">
              Try Again
            </button>
            <button onClick={this.handleReload} className="btn-primary text-sm px-4 py-2 flex items-center gap-2">
              <RefreshCw size={14} />
              Reload
            </button>
          </div>
        </div>
      </div>
    );
  }
}
