import { Injectable, signal } from '@angular/core';
import { ErrorSeverity } from '../models/app-error';

export interface NotificationOptions {
  message: string;
  severity?: ErrorSeverity;
  action?: string;
  duration?: number;
}

export interface Toast {
  id: number;
  message: string;
  severity: string;
  /** 0 = sticky (manual dismiss only) */
  duration: number;
}

/**
 * Lightweight toast notifications — a drop-in replacement for the old
 * MatSnackBar-based service. Public API (show/success/warning/error/critical)
 * is unchanged so callers don't change. Rendered by <app-toast-host> in the
 * app root.
 */
@Injectable({ providedIn: 'root' })
export class NotificationService {
  /** Active toasts, read by the toast host. */
  readonly toasts = signal<Toast[]>([]);
  private seq = 0;

  show(options: NotificationOptions): void {
    const severity = options.severity || 'error';
    const duration = options.duration ?? (severity === 'critical' ? 0 : 5000);
    this.push(options.message, severity, duration);
  }

  success(message: string, duration = 3000): void {
    this.push(message, 'success', duration);
  }

  warning(message: string, _action?: string): void {
    this.push(message, 'warning', 5000);
  }

  error(message: string, _action?: string): void {
    this.push(message, 'error', 5000);
  }

  critical(message: string, _action?: string): void {
    // Don't auto-dismiss critical errors.
    this.push(message, 'critical', 0);
  }

  dismiss(id: number): void {
    this.toasts.update(list => list.filter(t => t.id !== id));
  }

  private push(message: string, severity: string, duration: number): void {
    const id = ++this.seq;
    this.toasts.update(list => [...list, { id, message, severity, duration }]);
    if (duration > 0) {
      setTimeout(() => this.dismiss(id), duration);
    }
  }
}
