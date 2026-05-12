import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { invoke } from '@tauri-apps/api/core';
import { License } from '../models/license.model';

/**
 * Cached license state — once verified, skip re-checking on every navigation.
 * Reset on page reload (full app restart).
 */
let licenseVerified = false;

/**
 * Route guard that redirects to /activation when no license is activated.
 * Applied to all routes except /activation itself.
 * Caches the result so subsequent navigations don't re-invoke the backend.
 */
export const initGuard: CanActivateFn = async () => {
  if (licenseVerified) {
    return true;
  }

  const router = inject(Router);

  try {
    const license = await invoke<License | null>('get_license');
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
