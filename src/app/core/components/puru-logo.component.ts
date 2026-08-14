import { Component, Input } from '@angular/core';

/**
 * Puru Labs typographic logo. Renders crisply at any size — no bitmap assets.
 *
 * - `full`   → `puru labs.` + "PRIVATE LIMITED" tagline (legal lockup, app boot)
 * - `normal` → `puru labs.` (header, dashboard)
 * - `mark`   → `puru.` short mark (tiles, tight spaces)
 *
 * `theme="dark"` is for dark surfaces (the navy sidebar): white "puru",
 * lighter-blue "labs", brighter red dot.
 */
@Component({
  selector: 'puru-logo',
  standalone: true,
  template: `
    <span class="puru-logo" [class.dark]="theme === 'dark'" [style.font-size.px]="size">
      <span class="lock">
        <span class="puru">puru</span>
        @if (variant !== 'mark') {
          <span class="labs">labs</span>
        }
        <span class="dot">.</span>
      </span>
      @if (variant === 'full') {
        <span class="tagline">private limited</span>
      }
    </span>
  `,
  styles: [`
    .puru-logo {
      display: inline-flex;
      flex-direction: column;
      align-items: center;
      font-family: var(--font-brand);
      line-height: 1;
      gap: 0.22em;
    }
    .lock { display: inline-flex; align-items: baseline; }
    .puru { font-weight: 500; color: var(--brand-ink); letter-spacing: -0.02em; }
    .labs { font-weight: 500; color: var(--brand-blue); letter-spacing: -0.02em; margin-left: 0.2em; }
    .dot  { font-weight: 700; color: var(--brand-red); }
    .tagline {
      font-family: var(--font-support);
      font-size: 0.2em;
      font-weight: 400;
      color: #afafaf;
      letter-spacing: 0.9em;
      text-transform: uppercase;
      white-space: nowrap;
      padding-left: 0.9em; /* offset the trailing letter-spacing so it reads centered */
    }
    .dark .puru { color: #ffffff; }
    .dark .labs { color: var(--brand-blue-2); }
    .dark .dot  { color: var(--brand-red-light); }
    .dark .tagline { color: #6b7c8e; }
  `],
})
export class PuruLogoComponent {
  @Input() variant: 'full' | 'normal' | 'mark' = 'normal';
  @Input() theme: 'light' | 'dark' = 'light';
  /** Wordmark font-size in px (the tagline/dot scale from it). */
  @Input() size = 22;
}
