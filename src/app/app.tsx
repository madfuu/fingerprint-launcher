/** Product name is a brand, not translatable copy. */
const APP_NAME = 'Fingerprint Launcher';

/**
 * P0 placeholder. P5 replaces this with the provider stack, bootstrap
 * (settings → i18n / theme → showMainWindow) and the router.
 */
export function App() {
  return (
    <main className="flex h-screen w-full flex-col items-center justify-center gap-3 bg-background text-foreground">
      <img src="/logo.svg" alt={APP_NAME} className="size-14" />
      <h1 className="text-sm font-semibold tracking-tight">{APP_NAME}</h1>
    </main>
  );
}
