import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { Badge } from '@/components/ui/badge';

interface PagePlaceholderProps {
  icon: LucideIcon;
  title: string;
  description: string;
  children?: ReactNode;
}

/** Stand-in for a page built in a later phase: its header bar and a summary card. */
export function PagePlaceholder({
  icon: Icon,
  title,
  description,
  children,
}: PagePlaceholderProps) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between border-b border-border bg-background/10 px-4 py-2 backdrop-blur-2xl">
        <div className="flex h-8 items-center gap-2 text-xs font-semibold">
          <Icon className="size-3.5 text-primary" />
          {title}
        </div>
      </header>
      <div className="flex flex-1 items-center justify-center p-8">
        <section className="relative w-full max-w-md space-y-3 overflow-hidden rounded-lg border border-border bg-background/10 p-5 backdrop-blur-2xl">
          <Icon
            aria-hidden
            className="pointer-events-none absolute -right-10 -bottom-10 size-40 text-muted-foreground/5"
          />
          <div className="flex items-center justify-between gap-4">
            <h1 className="text-sm font-semibold">{title}</h1>
            <Badge
              variant="secondary"
              className="text-[10px] font-semibold tracking-wider uppercase"
            >
              {t('placeholder.comingSoon')}
            </Badge>
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">{description}</p>
          {children}
          <p className="text-[11px] text-muted-foreground/80">{t('placeholder.note')}</p>
        </section>
      </div>
    </div>
  );
}
