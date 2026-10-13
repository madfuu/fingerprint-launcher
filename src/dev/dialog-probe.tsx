/**
 * Dev-only: the app's command dialog, opened over the running app by the Playwright scripts
 * (`scripts/ui-check.mjs`, and the screenshots' "dialog" variant). It is the palette that P7
 * wires to Ctrl+K, built from the app's own `CommandDialog`, so what the scripts exercise is a
 * real modal dialog of this app.
 *
 * The app never imports `src/dev` (ESLint enforces it); the dev server serves it to the
 * scripts on demand, and production bundles contain none of it.
 */
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { NAV } from '@/app/layout/nav';
import {
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';

/** Closes itself on dismissal and tells the mounter, which then unmounts it. */
export function DialogProbe({ onClosed }: { onClosed: () => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(true);
  return (
    <CommandDialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          onClosed();
        }
      }}
    >
      <CommandInput placeholder={t('ui.commandPaletteDescription')} />
      <CommandList>
        <CommandGroup heading={t('ui.commandPalette')}>
          {Object.values(NAV).map(({ key, icon: Icon }) => (
            <CommandItem key={key} className="text-xs">
              <Icon />
              {t(`nav.${key}`)}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
