import { Component, type ErrorInfo, type ReactNode } from 'react';

import { reportBootFailure } from '@/app/boot-status';
import { RenderError } from '@/app/fatal-error';
import { toAppError } from '@/lib/bridge';

interface State {
  failed: boolean;
  error: unknown;
}

/**
 * Shows {@link RenderError} instead of an empty window when rendering throws. A throw during
 * the first render is a failed boot: `componentDidCatch` runs in that commit, before the
 * bootstrap's ready signal, so `showMainWindow` reports it.
 */
export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { failed: false, error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { failed: true, error };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error('render failed', error, info.componentStack);
    reportBootFailure(`render: ${toAppError(error).message}`);
  }

  render() {
    return this.state.failed ? <RenderError error={this.state.error} /> : this.props.children;
  }
}
