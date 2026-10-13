import { RotateCw, TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { TitlebarFrame } from '@/app/layout/app-titlebar';
import { Button } from '@/components/ui/button';
import { localizeError } from '@/i18n/errors';
import { toAppError } from '@/lib/bridge';

/** The startup read of the settings failed: the error code's message and the raw detail. */
export function SettingsReadError({ error }: { error: unknown }) {
  const { t } = useTranslation();
  const { code, message, detail } = localizeError(t, error);
  return (
    <FatalError summary={`${t('fatal.settingsFailed')} ${message}`} detail={`${code}: ${detail}`} />
  );
}

/** Rendering threw (see `AppErrorBoundary`). */
export function RenderError({ error }: { error: unknown }) {
  const { t } = useTranslation();
  return <FatalError summary={t('fatal.renderFailed')} detail={toAppError(error).message} />;
}

/**
 * Full-window error with the raw detail and a reload button: never a blank window. The bare
 * title bar keeps the frameless window movable and closable.
 */
function FatalError({ summary, detail }: { summary: string; detail: string }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-screen w-full flex-col bg-background text-foreground">
      <TitlebarFrame />
      <main className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-8">
        <section
          role="alert"
          className="w-full max-w-lg space-y-4 rounded-lg border border-border bg-card p-5 shadow-sm"
        >
          <div className="flex items-center gap-2">
            <TriangleAlert className="size-4 text-destructive" />
            <h1 className="text-sm font-semibold">{t('fatal.title')}</h1>
          </div>
          <p className="text-xs leading-relaxed">{summary}</p>
          <div className="space-y-1.5">
            <div className="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
              {t('fatal.detail')}
            </div>
            <pre className="max-h-48 overflow-auto rounded-md bg-muted p-3 font-mono text-[11px] break-all whitespace-pre-wrap text-muted-foreground">
              {detail}
            </pre>
          </div>
          <Button size="sm" onClick={() => window.location.reload()}>
            <RotateCw />
            {t('actions.reload')}
          </Button>
        </section>
      </main>
    </div>
  );
}
