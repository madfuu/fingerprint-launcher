import { BrowserRouter } from 'react-router';

import { AppRoutes } from '@/app/routes';
import { ThemeProvider } from '@/components/theme-provider';
import { Toaster } from '@/components/ui/sonner';
import { TooltipProvider } from '@/components/ui/tooltip';

/** Provider stack and router; rendered by the bootstrap once settings, i18n and theme are set. */
export function App() {
  return (
    <ThemeProvider>
      <TooltipProvider delayDuration={300}>
        <BrowserRouter>
          <AppRoutes />
        </BrowserRouter>
        <Toaster />
      </TooltipProvider>
    </ThemeProvider>
  );
}
