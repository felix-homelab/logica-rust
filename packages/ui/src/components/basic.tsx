// Basic controls on Radix primitives (doc 08): styled by us with tokens only.
import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, Ref } from 'react';
import { useId } from 'react';
import { DropdownMenu, Switch as RSwitch, Tabs as RTabs, Tooltip as RTooltip } from 'radix-ui';

const cx = (...c: (string | false | undefined)[]): string => c.filter(Boolean).join(' ');

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: 'sm' | 'md';
  icon?: ReactNode;
  ref?: Ref<HTMLButtonElement>;
}

export function Button({ variant = 'secondary', size = 'md', icon, className, children, type = 'button', ...rest }: ButtonProps) {
  return (
    <button type={type} className={cx('lg-button', `lg-button--${variant}`, `lg-button--${size}`, className)} {...rest}>
      {icon && (
        <span className="lg-button__icon" aria-hidden="true">
          {icon}
        </span>
      )}
      {children}
    </button>
  );
}

/** Keys of a shortcut, such as ["Ctrl", "K"]. */
export function Kbd({ keys }: { keys: string[] }) {
  return (
    <span className="lg-kbd">
      {keys.map((k) => (
        <kbd key={k}>{k}</kbd>
      ))}
    </span>
  );
}

export const TooltipProvider = RTooltip.Provider;

export function Tooltip({ content, shortcut, children }: { content: ReactNode; shortcut?: string[]; children: ReactNode }) {
  return (
    <RTooltip.Root delayDuration={400}>
      <RTooltip.Trigger asChild>{children}</RTooltip.Trigger>
      <RTooltip.Portal>
        <RTooltip.Content className="lg-tooltip" sideOffset={6}>
          {content}
          {shortcut && <Kbd keys={shortcut} />}
        </RTooltip.Content>
      </RTooltip.Portal>
    </RTooltip.Root>
  );
}

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  /** Names the button for screen readers and shows as its tooltip. */
  label: string;
  icon: ReactNode;
  shortcut?: string[];
  pressed?: boolean;
  variant?: ButtonVariant;
}

export function IconButton({ label, icon, shortcut, pressed, variant = 'ghost', className, ...rest }: IconButtonProps) {
  return (
    <Tooltip content={label} {...(shortcut ? { shortcut } : {})}>
      <button
        type="button"
        aria-label={label}
        aria-pressed={pressed}
        className={cx('lg-button', `lg-button--${variant}`, 'lg-button--icon', className)}
        {...rest}
      >
        <span aria-hidden="true">{icon}</span>
      </button>
    </Tooltip>
  );
}

export type Tone = 'neutral' | 'accent' | 'agent' | 'danger' | 'warning' | 'success';

export function Badge({ tone = 'neutral', children, title }: { tone?: Tone; children: ReactNode; title?: string }) {
  return (
    <span className={cx('lg-badge', `lg-badge--${tone}`)} title={title}>
      {children}
    </span>
  );
}

/**
 * A state written as text with a small dot in its colour ("● capturing", "● 2 problems"): reads as
 * text, not as a coloured label, and the colour is never the only signal.
 */
export function Status({ tone = 'neutral', children, title, pulse }: { tone?: Tone; children: ReactNode; title?: string; pulse?: boolean }) {
  return (
    <span className={cx('lg-status', `lg-status--${tone}`, pulse && 'lg-status--pulse')} title={title}>
      <span className="lg-status__dot" aria-hidden="true" />
      {children}
    </span>
  );
}

export interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label: string;
  hint?: string;
  error?: string;
  /** Hides the label visually (it stays the field's accessible name). */
  hideLabel?: boolean;
}

export function TextField({ label, hint, error, hideLabel, className, id, ...rest }: TextFieldProps) {
  const auto = useId();
  const fieldId = id ?? auto;
  const describedBy = error ? `${fieldId}-error` : hint ? `${fieldId}-hint` : undefined;
  return (
    <div className={cx('lg-field', className)}>
      <label htmlFor={fieldId} className={cx('lg-field__label', hideLabel && 'lg-visually-hidden')}>
        {label}
      </label>
      <input id={fieldId} className="lg-input" aria-invalid={error ? true : undefined} aria-describedby={describedBy} {...rest} />
      {error ? (
        <p id={`${fieldId}-error`} className="lg-field__error" role="alert">
          {error}
        </p>
      ) : (
        hint && (
          <p id={`${fieldId}-hint`} className="lg-field__hint">
            {hint}
          </p>
        )
      )}
    </div>
  );
}

export function Switch({
  checked,
  onCheckedChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onCheckedChange: (v: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="lg-switch-row">
      <div>
        <label htmlFor={id} className="lg-switch-row__label">
          {label}
        </label>
        {description && <p className="lg-switch-row__description">{description}</p>}
      </div>
      <RSwitch.Root id={id} className="lg-switch" checked={checked} onCheckedChange={onCheckedChange} disabled={disabled}>
        <RSwitch.Thumb className="lg-switch__thumb" />
      </RSwitch.Root>
    </div>
  );
}

export const TabsRoot = RTabs.Root;

export function TabsList({ label, children }: { label: string; children: ReactNode }) {
  return (
    <RTabs.List className="lg-tabs__list" aria-label={label}>
      {children}
    </RTabs.List>
  );
}

export function TabsTrigger({ value, children }: { value: string; children: ReactNode }) {
  return (
    <RTabs.Trigger className="lg-tabs__trigger" value={value}>
      {children}
    </RTabs.Trigger>
  );
}

export function TabsContent({ value, children, className }: { value: string; children: ReactNode; className?: string }) {
  return (
    <RTabs.Content className={cx('lg-tabs__content', className)} value={value}>
      {children}
    </RTabs.Content>
  );
}

export function Menu({ trigger, children, label }: { trigger: ReactNode; children: ReactNode; label?: string }) {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild aria-label={label}>
        {trigger}
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="lg-menu" sideOffset={4} align="start">
          {children}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

export function MenuItem({
  onSelect,
  icon,
  shortcut,
  danger,
  disabled,
  children,
}: {
  onSelect: () => void;
  icon?: ReactNode;
  shortcut?: string[];
  danger?: boolean;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <DropdownMenu.Item className={cx('lg-menu__item', danger && 'lg-menu__item--danger')} onSelect={onSelect} disabled={disabled}>
      <span className="lg-menu__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="lg-menu__text">{children}</span>
      {shortcut && <Kbd keys={shortcut} />}
    </DropdownMenu.Item>
  );
}

export function MenuSeparator() {
  return <DropdownMenu.Separator className="lg-menu__separator" />;
}

export function Panel({
  title,
  actions,
  children,
  className,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cx('lg-panel', className)}>
      {(title || actions) && (
        <header className="lg-panel__header">
          {title && <h2 className="lg-panel__title">{title}</h2>}
          {actions && <div className="lg-panel__actions">{actions}</div>}
        </header>
      )}
      <div className="lg-panel__body">{children}</div>
    </section>
  );
}

export function EmptyState({ icon, title, children, actions }: { icon?: ReactNode; title: string; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="lg-empty">
      {icon && (
        <div className="lg-empty__icon" aria-hidden="true">
          {icon}
        </div>
      )}
      <p className="lg-empty__title">{title}</p>
      {children && <div className="lg-empty__text">{children}</div>}
      {actions && <div className="lg-empty__actions">{actions}</div>}
    </div>
  );
}
