import { Outlet } from 'react-router';

import { AppBackground } from '@/app/layout/app-background';
import { AppSidebar } from '@/app/layout/app-sidebar';
import { AppTitlebar } from '@/app/layout/app-titlebar';

/**
 * Frame of every page (research/simprint-ui-reference.md §3): the background canvas behind a
 * content layer with the title bar on top, then the sidebar and the page side by side. Pages
 * fill the remaining area with flex; nothing assumes the title bar's height.
 */
export function AppLayout() {
  return (
    <div className="relative flex h-screen w-full min-w-0 flex-col overflow-hidden">
      <AppBackground />
      {/* Own stacking context: the sidebar's frosted backdrop (z -10) stays above the canvas. */}
      <div className="relative isolate flex h-full w-full min-w-0 flex-col overflow-hidden">
        <AppTitlebar />
        <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
          <AppSidebar />
          <main className="flex min-w-0 flex-1 flex-col overflow-hidden border-l border-sidebar-border/80">
            <div className="min-h-0 min-w-0 flex-1 overflow-hidden">
              <Outlet />
            </div>
          </main>
        </div>
      </div>
    </div>
  );
}
