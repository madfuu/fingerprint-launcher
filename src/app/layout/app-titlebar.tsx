/**
 * The window's title bar (research/simprint-ui-reference.md §5): brand, breadcrumb, status
 * (theme and language) and window controls, 48 px high. The whole bar drags the window except
 * its buttons (`data-tauri-drag-region="deep"`: Tauri's script skips clickable elements), and
 * double-clicking it maximizes.
 *
 * macOS keeps its native traffic lights over the bar (design §7): our window controls are
 * hidden there and the brand leaves room for the lights.
 */
import {
  ChevronRight,
  Copy,
  GripVertical,
  House,
  Languages,
  Minus,
  Monitor,
  Moon,
  Square,
  Sun,
  X,
  type LucideIcon,
} from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router';
import { cn } from 'cn';

import { useHostOs } from '@/app/app-info';
import { APP_LOGO, APP_NAME } from '@/app/brand';
import { currentPage } from '@/app/layout/nav';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useUpdateSettings } from '@/features/settings/hooks/use-update-settings';
import { useSettingsStore } from '@/features/settings/store';
import { useWindowMaximized } from '@/hooks/use-window-maximized';
import { locales } from '@/i18n/registry';
import type { Locale, Theme } from '@/lib/bindings';
import { closeWindow, minimizeWindow, toggleMaximizeWindow } from '@/lib/bridge';
import { TITLEBAR_ATTRIBUTE } from '@/lib/titlebar';

const THEME_ICONS: Record<Theme, LucideIcon> = { light: Sun, dark: Moon, system: Monitor };
const THEMES = Object.keys(THEME_ICONS) as Theme[];

/** Small square buttons of the status slot. */
const STATUS_BUTTON =
  'flex size-8 cursor-pointer items-center justify-center rounded-sm text-muted-foreground/80 outline-none transition-colors duration-200 hover:bg-accent/80 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 data-[state=open]:bg-accent/80 data-[state=open]:text-foreground';

/** The bar of the running app: every slot. Must be rendered inside the router. */
export function AppTitlebar() {
  return <TitlebarFrame navigation={<Breadcrumb />} status={<StatusSlot />} />;
}

interface TitlebarFrameProps {
  navigation?: ReactNode;
  status?: ReactNode;
}

/**
 * Brand and window controls around optional middle slots. Error screens use it bare: they
 * render without the router and settings, but the window must still be movable and closable.
 */
export function TitlebarFrame({ navigation, status }: TitlebarFrameProps) {
  const macos = useHostOs() === 'macos';
  return (
    <header
      {...{ [TITLEBAR_ATTRIBUTE]: '' }}
      data-tauri-drag-region="deep"
      // Radix turns pointer events off on <body> while a modal dialog is open; the title bar
      // keeps them (see lib/titlebar.ts).
      className="pointer-events-auto flex h-12 shrink-0 items-center justify-between border-b border-border/80 bg-background select-none"
    >
      <div className="flex h-full min-w-0 items-center">
        <div
          className={cn(
            'flex h-full shrink-0 items-center gap-3 border-r border-border/80 pr-5',
            macos ? 'pl-[78px]' : 'pl-5'
          )}
        >
          <img src={APP_LOGO} alt="" draggable={false} className="size-7" />
          <span className="text-xs font-semibold tracking-tight text-foreground">{APP_NAME}</span>
        </div>
        {navigation}
      </div>
      <div className={cn('flex h-full shrink-0 items-center', macos && 'pr-2')}>
        {status}
        {!macos && <WindowControls />}
      </div>
    </header>
  );
}

/** Home › current page. */
function Breadcrumb() {
  const { t } = useTranslation();
  const { t: tl } = useTranslation('layout');
  const { pathname } = useLocation();
  const page = currentPage(pathname);
  const PageIcon = page?.icon;
  const label = tl('titlebar.breadcrumb');
  return (
    <nav
      aria-label={label}
      className="flex min-w-0 items-center gap-2.5 px-5 text-xs text-muted-foreground"
    >
      <House aria-hidden className="size-3 shrink-0 opacity-70" />
      {page && PageIcon && (
        <>
          <ChevronRight aria-hidden className="size-3 shrink-0 opacity-50" />
          <PageIcon aria-hidden className="size-3 shrink-0 opacity-70" />
          <span className="truncate">{t(`nav.${page.key}`)}</span>
        </>
      )}
    </nav>
  );
}

function StatusSlot() {
  return (
    <div className="mx-2 flex items-center gap-1">
      <ThemeMenu />
      <LanguageMenu />
    </div>
  );
}

function ThemeMenu() {
  const { t } = useTranslation();
  const { t: tl } = useTranslation('layout');
  const theme = useSettingsStore((state) => state.settings?.theme ?? 'system');
  const save = useUpdateSettings();
  const Icon = THEME_ICONS[theme];
  const label = tl('titlebar.theme');
  return (
    <DropdownMenu>
      <DropdownMenuTrigger aria-label={label} title={label} className={STATUS_BUTTON}>
        <Icon className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-36">
        <MenuHeading>{label}</MenuHeading>
        <DropdownMenuRadioGroup
          value={theme}
          onValueChange={(value) => {
            const picked = THEMES.find((candidate) => candidate === value);
            if (picked) {
              save({ theme: picked });
            }
          }}
        >
          {THEMES.map((value) => {
            const ItemIcon = THEME_ICONS[value];
            return (
              <DropdownMenuRadioItem key={value} value={value} className="text-xs">
                <ItemIcon />
                {t(`theme.${value}`)}
              </DropdownMenuRadioItem>
            );
          })}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function LanguageMenu() {
  const { t } = useTranslation();
  const { t: tl } = useTranslation('layout');
  const locale = useSettingsStore((state) => state.settings?.locale ?? 'zh-CN');
  const save = useUpdateSettings();
  const label = tl('titlebar.language');
  return (
    <DropdownMenu>
      <DropdownMenuTrigger aria-label={label} title={label} className={STATUS_BUTTON}>
        <Languages className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-36">
        <MenuHeading>{label}</MenuHeading>
        <DropdownMenuRadioGroup
          value={locale}
          onValueChange={(value) => {
            const picked = locales.find((candidate: Locale) => candidate === value);
            if (picked) {
              save({ locale: picked });
            }
          }}
        >
          {locales.map((value) => (
            <DropdownMenuRadioItem key={value} value={value} lang={value} className="text-xs">
              {t(`language.${value}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function MenuHeading({ children }: { children: ReactNode }) {
  return (
    <DropdownMenuLabel className="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
      {children}
    </DropdownMenuLabel>
  );
}

/** Drag handle, minimize, maximize / restore, close (not on macOS). */
function WindowControls() {
  const { t } = useTranslation('layout');
  const maximized = useWindowMaximized();
  const run = (action: () => Promise<void>) => () => {
    // Only fails when a window permission is missing from the capabilities: a build defect,
    // nothing the user can act on. The error names the command.
    action().catch((error: unknown) => console.error('window control failed', error));
  };
  const resizeLabel = maximized ? t('titlebar.restore') : t('titlebar.maximize');
  return (
    <div className="flex h-full items-center">
      {/* No handler: it is part of the title bar's drag region. */}
      <div
        title={t('titlebar.drag')}
        className="flex h-full w-12 cursor-grab items-center justify-center border-l border-border/80 text-muted-foreground/50 hover:text-muted-foreground/80 active:cursor-grabbing"
      >
        <GripVertical aria-hidden className="size-4" />
      </div>
      <div className="flex h-full items-center border-l border-border/80">
        <WindowButton
          data-window-control="minimize"
          label={t('titlebar.minimize')}
          onClick={run(minimizeWindow)}
        >
          <Minus className="size-4" />
        </WindowButton>
        <WindowButton
          data-window-control="toggle-maximize"
          data-maximized={maximized}
          label={resizeLabel}
          onClick={run(toggleMaximizeWindow)}
        >
          {maximized ? (
            // Two overlapping squares, the back one top right: the usual "restore" glyph.
            <Copy className="size-3.5 -scale-x-100" />
          ) : (
            <Square className="size-3.5" />
          )}
        </WindowButton>
        <WindowButton
          data-window-control="close"
          label={t('titlebar.close')}
          destructive
          onClick={run(closeWindow)}
        >
          <X className="size-4" />
        </WindowButton>
      </div>
    </div>
  );
}

interface WindowButtonProps extends Omit<ComponentProps<'button'>, 'type'> {
  label: string;
  destructive?: boolean;
}

function WindowButton({ label, destructive = false, className, ...props }: WindowButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cn(
        'flex h-full w-12 cursor-pointer items-center justify-center text-muted-foreground/80 outline-none transition-colors duration-200 ease-in-out focus-visible:bg-accent/80 [&_svg]:pointer-events-none',
        destructive
          ? 'hover:bg-destructive/90 hover:text-destructive-foreground'
          : 'hover:bg-accent/80 hover:text-foreground',
        className
      )}
      {...props}
    />
  );
}
