// Small shared pieces: links, page header, loading/error/empty states.

import { type AnchorHTMLAttributes, type ReactNode } from 'react';
import {
  BUTTON_SIZES,
  BUTTON_VARIANTS,
  Button,
  EmptyState,
  Hint,
  Icon,
  ICONS,
  Nyu,
  Spinner,
  type ButtonSize,
  type ButtonVariant,
  type NyuMood,
} from '@uwusuite/design';
import type { LucideIcon } from 'lucide-react';
import { useT } from '../lib/i18n';
import { linkClick } from '../lib/router';

/** An in-app link: plain clicks navigate without reloading. */
export function Link({
  href,
  onClick,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  return (
    <a
      href={href}
      onClick={(e) => {
        onClick?.(e);
        linkClick(e);
      }}
      {...rest}
    />
  );
}

/** A link that looks like the design system's Button: one control, not a button inside a link. */
export function ButtonLink({
  variant = 'secondary',
  size = 'md',
  icon,
  className = '',
  children,
  external = false,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: LucideIcon;
  /** A full page load (sign-in), not an in-app route. */
  external?: boolean;
}) {
  const className_ = `inline-flex shrink-0 items-center justify-center gap-2 rounded-full font-semibold whitespace-nowrap no-underline transition-[background,box-shadow,transform,border-color] duration-150 focus-visible:shadow-focus active:scale-[0.97] phone:h-auto phone:min-h-10 phone:whitespace-normal ${BUTTON_VARIANTS[variant]} ${BUTTON_SIZES[size]} ${className}`;
  const inner = (
    <>
      {icon && <Icon icon={icon} strokeWidth={2.2} />}
      {children}
    </>
  );
  return external ? (
    <a className={className_} {...rest}>
      {inner}
    </a>
  ) : (
    <Link className={className_} {...rest}>
      {inner}
    </Link>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
  back,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  back?: { href: string; label: string };
}) {
  return (
    <header className="mb-6 flex flex-wrap items-end justify-between gap-3">
      <div className="min-w-0">
        {back && (
          <Link
            href={back.href}
            className="mb-1 inline-flex items-center gap-1 text-meta text-muted no-underline hover:text-ink"
          >
            <Icon icon={ICONS.back} size="xs" /> {back.label}
          </Link>
        )}
        <h1 className="m-0 text-title font-bold break-words">{title}</h1>
        {subtitle && <div className="mt-1 text-meta text-muted">{subtitle}</div>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}

export function Loading({ label }: { label?: string }) {
  const { t } = useT();
  return (
    <div className="flex items-center gap-2 p-6 text-muted" role="status">
      <Spinner /> {label ?? t('common.loading')}
    </div>
  );
}
export function ErrorBox({ error, retry }: { error: Error; retry?: () => void }) {
  const { t } = useT();
  return (
    <Hint tone="danger" className="flex flex-wrap items-center justify-between gap-2">
      <span>{error.message}</span>
      {retry && (
        <Button size="sm" variant="secondary" icon={ICONS.refresh} onClick={retry}>
          {t('common.retry')}
        </Button>
      )}
    </Hint>
  );
}

export function Empty({
  title,
  body,
  mood = 'sleepy',
  action,
}: {
  title: string;
  body?: ReactNode;
  mood?: NyuMood;
  action?: ReactNode;
}) {
  return (
    <EmptyState
      compact
      art={<Nyu shell="box" mood={mood} size={64} title="" />}
      title={title}
      body={body}
      action={action}
    />
  );
}
