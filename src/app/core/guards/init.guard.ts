import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { invoke } from '@tauri-apps/api/core';
import { License } from '../models/license.model';

/**
 * Route guard that redirects to /activation when no license is activated.
 * Applied to all routes except /activation itself.
 */
export const initGuard: CanActivateFn = async () => {
  const router = inject(Router);

  try {
    const license = await invoke<License | null>('get_license');
    if (license) {
      return true;
    }
  } catch {
    // If the command fails, treat as no license
  }

  return router.createUrlTree(['/activation']);
};
