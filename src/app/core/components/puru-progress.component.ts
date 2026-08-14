import { Component, Input } from '@angular/core';
import { CommonModule } from '@angular/common';

type ProgressState = 'active' | 'complete' | 'hold' | 'fail' | 'indeterminate';

/**
 * Puru progress bar — the logo's red dot becomes the progress head, so loading
 * reads as Puru without a separate spinner. Blue fills the track; semantic
 * colors take over on complete (green), hold (amber), and fail (red).
 *
 * Usage: `<puru-progress [value]="68" state="active"></puru-progress>`
 */
@Component({
  selector: 'puru-progress',
  standalone: true,
  imports: [CommonModule],
  template: `
    <div class="puru-progress" [ngClass]="state">
      @if (state === 'indeterminate') {
        <div class="track" [style.height.px]="height">
          <div class="sweep">
            <div class="seg"></div>
            <div class="head"></div>
          </div>
        </div>
      } @else {
        <div class="track" [style.height.px]="height">
          <div class="fill" [style.width.%]="clamped"></div>
          @if (state !== 'complete') {
            <div class="head" [style.left.%]="clamped">
              @if (state === 'fail') {
                <span class="material-icons x">close</span>
              }
            </div>
          }
        </div>
      }
    </div>
  `,
  styles: [`
    .puru-progress { width: 100%; }
    .track {
      position: relative;
      width: 100%;
      border-radius: 999px;
      background: var(--brand-track);
      overflow: visible;
    }
    .fill {
      position: absolute;
      left: 0; top: 0; bottom: 0;
      border-radius: 999px;
      background: var(--brand-blue);
      transition: width 0.35s ease;
    }
    .head {
      position: absolute;
      top: 50%;
      width: 1.6em;
      height: 1.6em;
      border-radius: 50%;
      background: var(--brand-red);
      border: 0.3em solid var(--bg-card);
      box-sizing: border-box;
      transform: translate(-50%, -50%);
      transition: left 0.35s ease;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 10px; /* base for the em-sized dot; scales with track height below */
    }
    .head .x { font-size: 0.9em; color: #fff; line-height: 1; }

    /* Dot scales with the track: ~2× the track height reads right. */
    .track { font-size: inherit; }

    /* ── States ── */
    .complete .fill { width: 100% !important; background: var(--brand-green); }
    .hold .fill { background: var(--brand-amber); }
    .hold .head { background: var(--brand-amber); }
    .fail .fill { background: var(--brand-red-light); }
    .fail .head { background: var(--brand-red-light); }

    /* ── Indeterminate ── */
    .track { }
    .indeterminate .track { overflow: hidden; }
    .sweep {
      position: absolute;
      top: 0; bottom: 0; left: 0;
      width: 30%;
      display: flex;
      align-items: center;
      animation: puru-sweep 1.6s ease-in-out infinite;
    }
    .sweep .seg { flex: 1; height: 100%; border-radius: 999px; background: var(--brand-blue); }
    .indeterminate .head {
      position: static;
      transform: none;
      width: 1.2em; height: 1.2em;
      border: none;
      margin-left: -0.6em;
      font-size: 10px;
    }
    @keyframes puru-sweep {
      0% { transform: translateX(-110%); }
      100% { transform: translateX(430%); }
    }
  `],
})
export class PuruProgressComponent {
  /** 0–100. Ignored for `indeterminate`. */
  @Input() value = 0;
  @Input() state: ProgressState = 'active';
  /** Track height in px (the dot head scales from it). */
  @Input() height = 8;

  get clamped(): number {
    return Math.max(0, Math.min(100, this.value || 0));
  }
}
