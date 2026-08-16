import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { ConnectionService } from '../services/connection.service';

/**
 * Guards the configuration screens (Settings, Compose, Setup, Master Data,
 * Shell). Blocks them by URL when the local system is in production mode, and
 * blocks the Compose screen specifically in native (JAR) mode where it's
 * meaningless. Redirects to the dashboard. Only enforced for the local machine;
 * a remote session leaves these to the remote's own policy.
 */
export const configGuard: CanActivateFn = async (route) => {
  const router = inject(Router);
  const conn = inject(ConnectionService);

  // Only gate the local machine; remote management is unaffected here.
  if (conn.isRemote()) return true;

  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const cfg = await invoke<{ deployment_mode?: string; production_mode?: boolean }>('get_config');

    if (cfg?.production_mode) {
      return router.parseUrl('/dashboard');
    }
    if (route.routeConfig?.path === 'compose' && cfg?.deployment_mode === 'native') {
      return router.parseUrl('/dashboard');
    }
    return true;
  } catch {
    // If config can't be read, don't lock the user out.
    return true;
  }
};
