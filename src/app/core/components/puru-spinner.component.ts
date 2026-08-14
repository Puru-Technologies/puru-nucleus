import { Component, Input } from '@angular/core';

/**
 * Puru branded loader — variant "2d" from the design: the `puru.` wordmark
 * types itself in over a ghost, ending on the red dot, then resets.
 * Splash / full-screen boot material. For tiny inline spinners use the global
 * `.spinner` class instead (blue arc + red-dot head).
 */
@Component({
  selector: 'puru-spinner',
  standalone: true,
  template: `
    <span class="puru-type" [class.dark]="theme === 'dark'" [style.font-size.px]="size">
      <span class="ghost"><span class="w">puru</span><span class="d">.</span></span>
      <span class="ink"><span class="w">puru</span><span class="d">.</span></span>
    </span>
  `,
  styles: [`
    .puru-type {
      position: relative;
      display: inline-flex;
      font-family: var(--font-brand);
      line-height: 1;
    }
    .puru-type .w { font-weight: 500; letter-spacing: -0.02em; }
    .puru-type .d { font-weight: 700; }
    .ghost { display: inline-flex; align-items: baseline; }
    .ghost .w { color: #dfe8f4; }
    .ghost .d { color: #f6dada; }
    .ink {
      position: absolute;
      left: 0;
      top: 0;
      display: inline-flex;
      align-items: baseline;
      overflow: hidden;
      white-space: nowrap;
      animation: puru-type 2s steps(5, end) infinite;
    }
    .ink .w { color: var(--brand-ink); }
    .ink .d { color: var(--brand-red); }
    .dark .ghost .w { color: rgba(255, 255, 255, 0.18); }
    .dark .ghost .d { color: rgba(255, 94, 94, 0.28); }
    .dark .ink .w { color: #ffffff; }
    .dark .ink .d { color: var(--brand-red-light); }
    @keyframes puru-type {
      0%, 10% { width: 0; }
      55%, 80% { width: 100%; }
      100% { width: 100%; opacity: 0; }
    }
  `],
})
export class PuruSpinnerComponent {
  @Input() theme: 'light' | 'dark' = 'light';
  /** Wordmark font-size in px. */
  @Input() size = 34;
}
