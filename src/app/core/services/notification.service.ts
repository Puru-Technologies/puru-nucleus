import { Injectable } from '@angular/core';
import { MatSnackBar, MatSnackBarConfig } from '@angular/material/snack-bar';
import { ErrorSeverity } from '../models/app-error';

export interface NotificationOptions {
  message: string;
  severity?: ErrorSeverity;
  action?: string;
  duration?: number;
}

@Injectable({
  providedIn: 'root'
})
export class NotificationService {
  constructor(private snackBar: MatSnackBar) {}

  show(options: NotificationOptions): void {
    const severity = options.severity || 'error';
    const duration = options.duration ?? (severity === 'critical' ? 0 : 5000);

    const config: MatSnackBarConfig = {
      duration,
      panelClass: `snackbar-${severity}`,
      horizontalPosition: 'end',
      verticalPosition: 'bottom'
    };

    this.snackBar.open(
      options.message,
      options.action || 'Dismiss',
      config
    );
  }

  success(message: string, duration = 3000): void {
    this.snackBar.open(message, 'OK', {
      duration,
      panelClass: 'snackbar-success',
      horizontalPosition: 'end',
      verticalPosition: 'bottom'
    });
  }

  warning(message: string, action?: string): void {
    this.show({
      message,
      severity: 'warning',
      action
    });
  }

  error(message: string, action?: string): void {
    this.show({
      message,
      severity: 'error',
      action
    });
  }

  critical(message: string, action?: string): void {
    this.show({
      message,
      severity: 'critical',
      action,
      duration: 0 // Don't auto-dismiss critical errors
    });
  }
}
