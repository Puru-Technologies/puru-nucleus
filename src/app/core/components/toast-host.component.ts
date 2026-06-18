import { Component, inject } from '@angular/core';
import { NotificationService } from '../services/notification.service';

/**
 * Renders active toasts (bottom-right stack). Mounted once in the app root.
 * Replaces MatSnackBar with a tiny, dependency-free toast.
 */
@Component({
  selector: 'app-toast-host',
  standalone: true,
  template: `
    <div class="toast-stack">
      @for (t of notify.toasts(); track t.id) {
        <div class="toast" [class]="'toast-' + t.severity">
          <span class="toast-msg">{{ t.message }}</span>
          <button class="toast-close" (click)="notify.dismiss(t.id)" aria-label="Dismiss">
            <span class="material-icons">close</span>
          </button>
        </div>
      }
    </div>
  `,
  styles: [`
    .toast-stack {
      position: fixed;
      bottom: 16px;
      right: 16px;
      z-index: 1000;
      display: flex;
      flex-direction: column;
      gap: 8px;
      max-width: 380px;
    }

    .toast {
      display: flex;
      align-items: flex-start;
      gap: 10px;
      padding: 10px 12px;
      border-radius: 8px;
      color: #fff;
      font-size: 0.85rem;
      line-height: 1.35;
      box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
      animation: toast-in 0.15s ease;
    }

    @keyframes toast-in {
      from { opacity: 0; transform: translateY(6px); }
      to   { opacity: 1; transform: translateY(0); }
    }

    .toast-msg { flex: 1; }

    .toast-close {
      background: transparent;
      border: none;
      color: rgba(255, 255, 255, 0.85);
      cursor: pointer;
      padding: 0;
      display: flex;
      align-items: center;

      .material-icons { font-size: 16px; }
      &:hover { color: #fff; }
    }

    .toast-success  { background: #059669; }
    .toast-warning  { background: #d97706; }
    .toast-error    { background: #dc2626; }
    .toast-critical { background: #991b1b; }
  `]
})
export class ToastHostComponent {
  notify = inject(NotificationService);
}
