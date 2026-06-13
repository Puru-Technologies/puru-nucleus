import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
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
 * Route guard that redirects to /activation when no license is activated.
 * Applied to all routes except /activation and /connect.
 * In remote mode the license comes from the connected daemon (GET /api/license);
 * in local mode it's the local Tauri command. Result is cached per navigation.
 */
export const initGuard: CanActivateFn = async () => {
  if (licenseVerified) {
    return true;
  }

  const router = inject(Router);
  const conn = inject(ConnectionService);
  const remote = inject(RemoteTransportService);

  try {
    const license = conn.isRemote()
      ? await remote.execute<License | null>('get_license')
      : await invoke<License | null>('get_license');
    if (license) {
      licenseVerified = true;
      return true;
    }
  } catch {
    // If the command fails, treat as no license
  }

  return router.createUrlTree(['/activation']);
};

/**
 * Call this after reset_activation to force re-check on next navigation.
 */
export function resetLicenseCache(): void {
  licenseVerified = false;
}
