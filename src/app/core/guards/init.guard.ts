import { inject } from '@angular/core';
import { ActivatedRouteSnapshot, CanActivateFn, Router } from '@angular/router';
import { invoke } from '@tauri-apps/api/core';
import { License } from '../models/license.model';
import { ConnectionService } from '../services/connection.service';
import { RemoteTransportService } from '../services/remote-transport';

/**
 * Cached license state — once verified, skip re-checking on every navigation.
 * Reset on page reload (full app restart) or when the connection changes.
 */
let licenseVerified = false;
/**
 * Cached setup-complete state (local mode only). Once we've seen it true, don't
 * re-hit the config for every navigation. Reset via resetLicenseCache().
 */
let setupCompleteVerified = false;

/**
 * Route guard that redirects to /activation when no license is activated, and
 * to /setup when setup hasn't been completed on the local machine. Applied to
 * every route except /activation, /connect, and /setup itself.
 *
 * In remote mode we only check license — setup-completion belongs to the
 * remote's own policy (its own daemon decides what its operators can see).
 */
export const initGuard: CanActivateFn = async (route: ActivatedRouteSnapshot) => {
  const router = inject(Router);
  const conn = inject(ConnectionService);
  const remote = inject(RemoteTransportService);

  // ── License gate ──────────────────────────────────────────────────────────
  if (!licenseVerified) {
    try {
      const license = conn.isRemote()
        ? await remote.execute<License | null>('get_license')
        : await invoke<License | null>('get_license');
      if (license) {
        licenseVerified = true;
      } else {
        return router.createUrlTree(['/activation']);
      }
    } catch {
      return router.createUrlTree(['/activation']);
    }
  }

  // ── Setup-complete gate (local only) ──────────────────────────────────────
  // Keep the Setup route itself reachable so the user can complete setup.
  const targetPath = route.routeConfig?.path;
  if (!conn.isRemote() && targetPath !== 'setup' && !setupCompleteVerified) {
    try {
      const cfg = await invoke<{ setup_completed?: boolean }>('get_config');
      if (cfg?.setup_completed) {
        setupCompleteVerified = true;
      } else {
        return router.createUrlTree(['/setup']);
      }
    } catch {
      // If we can't read the config, don't lock the user out of their app.
    }
  }

  return true;
};

/**
 * Call this after reset_activation to force re-check on next navigation.
 */
export function resetLicenseCache(): void {
  licenseVerified = false;
  setupCompleteVerified = false;
}
