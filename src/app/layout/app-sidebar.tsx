/**
 * Collapsible icon sidebar (research/simprint-ui-reference.md §6): 64 px collapsed, 232 px
 * expanded, over a frosted backdrop. From top to bottom: the "new profile" call to action, the
 * main entries, titled sections, the settings entry at the foot, and the collapse toggle that
 * sits just outside the bottom-right corner.
 *
 * The collapsed state is the `sidebarCollapsed` setting (collapsed by default). While the width
 * animates the entries are hidden, then slide back in, so labels never wrap mid-transition.
 */
import { ChevronLeft, ChevronRight, Globe } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router';
import { cn } from 'cn';

import { isNavActive, NAV, SIDEBAR, type NavEntry } from '@/app/layout/nav';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useUpdateSettings } from '@/features/settings/hooks/use-update-settings';
import { useSettingsStore } from '@/features/settings/store';

/** Matches the `duration-300` width transition. */
const RESIZE_MS = 300;

/** `aria-current` of the active entry. */
const CURRENT_PAGE = 'page';

export function AppSidebar() {
  const { t } = useTranslation('layout');
  const collapsed = useSettingsStore((state) => state.settings?.sidebarCollapsed ?? true);
  const save = useUpdateSettings();
  const { pathname } = useLocation();
  const [resizing, setResizing] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  const toggle = () => {
    setResizing(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setResizing(false), RESIZE_MS);
    save({ sidebarCollapsed: !collapsed });
  };

  const item = (entry: NavEntry) => (
    <NavItem
      key={entry.key}
      entry={entry}
      active={isNavActive(entry, pathname)}
      collapsed={collapsed}
    />
  );

  return (
    <aside
      data-collapsed={collapsed}
      className={cn(
        'relative flex h-full shrink-0 flex-col px-1 transition-[width] duration-300 ease-out',
        collapsed ? 'w-16' : 'w-58'
      )}
    >
      {/* Frosted backdrop over the window background; behind everything in the frame. */}
      <div aria-hidden className="absolute inset-0 -z-10 bg-background/10 backdrop-blur-2xl" />

      <div
        className={cn(
          'flex min-h-0 flex-1 flex-col',
          resizing ? 'opacity-0' : 'animate-[slideInRight_300ms_ease-in-out]'
        )}
      >
        <div className="m-1 mb-2 flex shrink-0 flex-col gap-1">
          <NewProfileButton collapsed={collapsed} />
          {SIDEBAR.top.map(item)}
          {collapsed && <div className="mx-3 border-b border-sidebar-border pt-2" />}
        </div>

        <nav aria-label={t('sidebar.label')} className="m-1 min-h-0 flex-1 overflow-y-auto">
          {SIDEBAR.sections.map((section, index) => (
            <section key={section.title} className={cn(!collapsed && 'mb-3')}>
              {index > 0 && collapsed && (
                <div className="mx-3 my-1 border-t border-sidebar-border" />
              )}
              {!collapsed && (
                <h2 className="px-3 py-1 text-[10px] font-semibold tracking-wider text-sidebar-foreground/50 uppercase">
                  {t(`sidebar.${section.title}`)}
                </h2>
              )}
              <div className="flex flex-col gap-1">{section.entries.map(item)}</div>
            </section>
          ))}
        </nav>

        <div className="m-1 flex shrink-0 flex-col gap-1">
          {collapsed && <div className="mx-3 mb-1 border-t border-sidebar-border" />}
          {SIDEBAR.bottom.map(item)}
        </div>
      </div>

      <CollapseToggle collapsed={collapsed} onToggle={toggle} />
    </aside>
  );
}

/** Shows `label` beside `children` while the sidebar is collapsed (labels are hidden then). */
function CollapsedTooltip({
  collapsed,
  label,
  children,
}: {
  collapsed: boolean;
  label: string;
  children: React.ReactElement;
}) {
  if (!collapsed) {
    return children;
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

/** Gradient call to action at the top: opens the editor for a new profile. */
function NewProfileButton({ collapsed }: { collapsed: boolean }) {
  const { t } = useTranslation();
  const { icon: Icon, to } = NAV.newProfile;
  const label = t('nav.newProfile');
  return (
    <CollapsedTooltip collapsed={collapsed} label={label}>
      <Link
        to={to}
        aria-label={collapsed ? label : undefined}
        className={cn(
          'relative mb-2 flex w-full items-center overflow-hidden rounded-lg bg-linear-to-r from-blue-500 via-blue-600 to-indigo-600 text-sidebar-primary-foreground shadow-sm transition-all duration-300 ease-in-out outline-none hover:from-blue-600 hover:via-blue-700 hover:to-indigo-700 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring/50',
          collapsed ? 'justify-center py-3.5' : 'gap-3 px-3 py-4'
        )}
      >
        {!collapsed && (
          <Globe
            aria-hidden
            className="pointer-events-none absolute -top-2 -right-2 size-16 text-sidebar-primary-foreground/10"
          />
        )}
        <Icon aria-hidden className="relative size-4 shrink-0" />
        {!collapsed && (
          <span className="relative h-4 truncate text-xs leading-4 font-semibold">{label}</span>
        )}
      </Link>
    </CollapsedTooltip>
  );
}

function NavItem({
  entry,
  active,
  collapsed,
}: {
  entry: NavEntry;
  active: boolean;
  collapsed: boolean;
}) {
  const { t } = useTranslation();
  const { icon: Icon, to, key } = entry;
  const label = t(`nav.${key}`);
  return (
    <CollapsedTooltip collapsed={collapsed} label={label}>
      <Link
        to={to}
        aria-current={active ? CURRENT_PAGE : undefined}
        aria-label={collapsed ? label : undefined}
        className={cn(
          'relative flex h-11 shrink-0 items-center rounded-lg px-3 transition-colors duration-200 outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
          collapsed ? 'w-11 justify-center' : 'w-full gap-3',
          active
            ? 'bg-primary/15 font-semibold text-primary'
            : 'text-sidebar-foreground/80 hover:bg-accent/60 hover:text-foreground'
        )}
      >
        {active && (
          // Marker on the item's right edge.
          <span
            aria-hidden
            className="absolute top-1/2 -right-1 h-6 w-1 -translate-y-1/2 rounded-2xl bg-primary"
          />
        )}
        <Icon aria-hidden className={cn('size-4 shrink-0', !active && 'opacity-80')} />
        {!collapsed && (
          <span
            className={cn(
              'h-4 truncate text-xs leading-4 font-semibold',
              !active && 'text-sidebar-foreground/90'
            )}
          >
            {label}
          </span>
        )}
      </Link>
    </CollapsedTooltip>
  );
}

/** Diagonal chevron just past the bottom-right corner, gently pulsing to be noticed. */
function CollapseToggle({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  const { t } = useTranslation('layout');
  const label = collapsed ? t('sidebar.expand') : t('sidebar.collapse');
  const Icon = collapsed ? ChevronRight : ChevronLeft;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          aria-expanded={!collapsed}
          onClick={onToggle}
          className="group absolute -right-6 bottom-0 z-20 flex size-6 animate-[pulse-gentle_2s_ease-in-out_infinite] cursor-pointer items-center justify-center text-sidebar-foreground/70 transition-colors duration-200 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <Icon
            aria-hidden
            className="size-6 -rotate-45 animate-[slide-hint_2s_ease-in-out_infinite] opacity-80 group-hover:opacity-100"
          />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
