/**
 * Mounts {@link DialogProbe} in its own React root (dev only, see `dialog-probe.tsx`). The
 * scripts call it through `import('/src/dev/mount-dialog-probe.tsx')`, which the dev server
 * resolves to the modules the app already loaded, so the dialog shares its React instance and
 * its initialized i18n.
 */
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import { DialogProbe } from '@/dev/dialog-probe';
import { i18n } from '@/i18n';

/** Time for the dialog's close animation before the root goes away. */
const UNMOUNT_DELAY_MS = 300;

export function openDialogProbe(): void {
  const host = document.createElement('div');
  host.dataset.dialogProbe = '';
  document.body.append(host);
  const root = createRoot(host);
  root.render(
    <I18nextProvider i18n={i18n}>
      <DialogProbe
        onClosed={() => {
          setTimeout(() => {
            root.unmount();
            host.remove();
          }, UNMOUNT_DELAY_MS);
        }}
      />
    </I18nextProvider>
  );
}
