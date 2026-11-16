import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Icon, type IconName } from "./Icon";
import type { Tone } from "../lib/status";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

export function Button({
  variant = "secondary",
  size,
  loading = false,
  icon,
  block = false,
  children,
  className = "",
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: "sm" | "lg";
  loading?: boolean;
  icon?: IconName;
  block?: boolean;
}) {
  const classes = [
    "btn",
    `btn--${variant}`,
    size ? `btn--${size}` : "",
    block ? "btn--block" : "",
    className,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button type="button" className={classes} disabled={disabled || loading} aria-busy={loading} {...rest}>
      {loading ? <span className="btn__loading" aria-hidden="true" /> : icon ? <Icon name={icon} size={16} /> : null}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  label,
  bordered = false,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon: IconName; label: string; bordered?: boolean }) {
  return (
    <button
      type="button"
      className={`icon-btn${bordered ? " icon-btn--bordered" : ""}${className ? ` ${className}` : ""}`}
      aria-label={label}
      title={label}
      {...rest}
    >
      <Icon name={icon} size={16} />
    </button>
  );
}

export function StatusPill({
  tone = "neutral",
  label,
  icon,
}: {
  tone?: Tone;
  label: string;
  icon?: IconName;
}) {
  return (
    <span className={`pill pill--${tone}`} role="status">
      <span className="pill__dot" aria-hidden="true" />
      <span>{label}</span>
      {icon ? <Icon name={icon} size={12} /> : null}
    </span>
  );
}

export function Card({
  title,
  subtitle,
  actions,
  children,
  className = "",
  bodyClassName = "",
  flush = false,
  footer,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  flush?: boolean;
  footer?: ReactNode;
}) {
  return (
    <section className={`card${className ? ` ${className}` : ""}`}>
      {title || actions ? (
        <header className="card__header">
          <div className="stack" style={{ gap: 0 }}>
            {title ? <h2 className="card__title">{title}</h2> : null}
            {subtitle ? <span className="card__subtitle">{subtitle}</span> : null}
          </div>
          {actions ? <div className="card__actions">{actions}</div> : null}
        </header>
      ) : null}
      <div className={`card__body${flush ? " card__body--flush" : ""}${bodyClassName ? ` ${bodyClassName}` : ""}`}>
        {children}
      </div>
      {footer ? <footer className="card__footer">{footer}</footer> : null}
    </section>
  );
}

export function StatCard({
  label,
  value,
  unit,
  note,
  alert = false,
  className = "",
  testId,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  note?: string;
  alert?: boolean;
  className?: string;
  testId?: string;
}) {
  return (
    <div className={`stat-card${alert ? " stat-card--alert" : ""}${className ? ` ${className}` : ""}`} data-testid={testId}>
      <span className="stat-card__label">{label}</span>
      <div className="stat-card__value-row">
        <span className="stat-card__value tabular">{value}</span>
        {unit ? <span className="stat-card__unit">{unit}</span> : null}
      </div>
      {note ? <span className="stat-card__note">{note}</span> : null}
    </div>
  );
}

export function Banner({
  tone = "info",
  title,
  children,
  actions,
  icon,
}: {
  tone?: "info" | "success" | "warning" | "danger";
  title?: string;
  children?: ReactNode;
  actions?: ReactNode;
  icon?: IconName;
}) {
  const resolved: IconName = icon ?? (tone === "danger" ? "alertTriangle" : tone === "success" ? "check" : tone === "warning" ? "alertTriangle" : "info");
  return (
    <div className={`banner banner--${tone}`} role={tone === "danger" ? "alert" : "status"}>
      <span className="banner__icon">
        <Icon name={resolved} size={16} />
      </span>
      <div className="banner__body">
        {title ? <span className="banner__title">{title}</span> : null}
        {children ? <span>{children}</span> : null}
      </div>
      {actions ? <div className="banner__actions">{actions}</div> : null}
    </div>
  );
}

export function EmptyState({
  icon = "info",
  title,
  text,
  actions,
}: {
  icon?: IconName;
  title: string;
  text?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <span className="empty-state__icon">
        <Icon name={icon} size={26} />
      </span>
      <span className="empty-state__title">{title}</span>
      {text ? <span className="empty-state__text">{text}</span> : null}
      {actions ? <div className="empty-state__actions">{actions}</div> : null}
    </div>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
  title?: string;
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
  className = "",
}: {
  options: SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  ariaLabel: string;
  className?: string;
}) {
  return (
    <div className={`segmented${className ? ` ${className}` : ""}`} role="group" aria-label={ariaLabel}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className="segmented__item"
          aria-pressed={option.value === value}
          disabled={option.disabled}
          title={option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function Field({
  label,
  hint,
  error,
  required,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  error?: string;
  required?: boolean;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div className="field">
      <label className={`field__label${required ? " field__label--required" : ""}`} htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? <span className="field__error" role="alert">{error}</span> : hint ? <span className="field__hint">{hint}</span> : null}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
  id,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  id?: string;
}) {
  return (
    <label className="checkbox" htmlFor={id}>
      <span className="switch">
        <input
          id={id}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span className="switch__track" aria-hidden="true" />
      </span>
      <span>{label}</span>
    </label>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
  badge,
}: {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  badge?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div className="page-heading">
        <div className="page-heading__title-row">
          <h1 className="page-title">{title}</h1>
          {badge}
        </div>
        {subtitle ? <p className="page-subtitle">{subtitle}</p> : null}
      </div>
      {actions ? <div className="page-actions">{actions}</div> : null}
    </header>
  );
}

export function Progress({ value, tone = "info" }: { value: number; tone?: "info" | "warning" | "danger" }) {
  const clamped = Math.max(0, Math.min(100, value));
  return (
    <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(clamped)}>
      <div className={`progress__fill${tone !== "info" ? ` progress__fill--${tone}` : ""}`} style={{ width: `${clamped}%` }} />
    </div>
  );
}
