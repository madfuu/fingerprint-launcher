import { Outlet } from 'react-router';

/**
 * Frame of every page. P6 adds the title bar, sidebar and background around the outlet
 * (research/simprint-ui-reference.md §3); until then pages fill the window.
 */
export function AppLayout() {
  return (
    <div className="relative flex h-screen w-full min-w-0 flex-col overflow-hidden">
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <Outlet />
      </main>
    </div>
  );
}
