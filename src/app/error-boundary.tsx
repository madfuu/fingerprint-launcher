import { Component, type ErrorInfo, type ReactNode } from 'react';

import { RenderError } from '@/app/fatal-error';

interface State {
  failed: boolean;
  error: unknown;
}

/** Shows {@link FatalError} instead of an empty window when rendering throws. */
export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { failed: false, error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { failed: true, error };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error('render failed', error, info.componentStack);
  }

  render() {
    return this.state.failed ? <RenderError error={this.state.error} /> : this.props.children;
  }
}
