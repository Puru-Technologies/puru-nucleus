import { Routes } from '@angular/router';
import { initGuard } from './core/guards/init.guard';
import { configGuard } from './core/guards/config.guard';

export const routes: Routes = [
  {
    path: '',
    redirectTo: 'dashboard',
    pathMatch: 'full'
  },
  {
    path: 'activation',
    loadComponent: () => import('./features/activation/activation.component').then(m => m.ActivationComponent)
  },
  {
    path: 'connect',
    loadComponent: () => import('./features/connect/connect.component').then(m => m.ConnectComponent)
  },
  {
    path: 'dashboard',
    canActivate: [initGuard],
    loadComponent: () => import('./features/dashboard/dashboard.component').then(m => m.DashboardComponent)
  },
  {
    path: 'services',
    canActivate: [initGuard],
    loadComponent: () => import('./features/services/services.component').then(m => m.ServicesComponent)
  },
  {
    path: 'logs',
    canActivate: [initGuard],
    loadComponent: () => import('./features/logs/logs.component').then(m => m.LogsComponent)
  },
  {
    path: 'backups',
    canActivate: [initGuard],
    loadComponent: () => import('./features/backups/backups.component').then(m => m.BackupsComponent)
  },
  {
    path: 'alerts',
    canActivate: [initGuard],
    loadComponent: () => import('./features/alerts/alerts.component').then(m => m.AlertsComponent)
  },
  {
    path: 'inbox',
    canActivate: [initGuard],
    loadComponent: () => import('./features/inbox/inbox.component').then(m => m.InboxComponent)
  },
  {
    path: 'updates',
    canActivate: [initGuard],
    loadComponent: () => import('./features/updates/updates.component').then(m => m.UpdatesComponent)
  },
  {
    path: 'remote-shell',
    canActivate: [initGuard, configGuard],
    loadComponent: () => import('./features/remote-shell/remote-shell.component').then(m => m.RemoteShellComponent)
  },
  {
    path: 'settings',
    canActivate: [initGuard, configGuard],
    loadComponent: () => import('./features/settings/settings.component').then(m => m.SettingsComponent)
  },
  {
    path: 'setup',
    canActivate: [initGuard, configGuard],
    loadComponent: () => import('./features/setup/setup.component').then(m => m.SetupComponent)
  },
  {
    path: 'compose',
    canActivate: [initGuard, configGuard],
    loadComponent: () => import('./features/compose/compose.component').then(m => m.ComposeComponent)
  },
  {
    path: 'master-data',
    canActivate: [initGuard, configGuard],
    loadComponent: () => import('./features/master-data/master-data.component').then(m => m.MasterDataComponent)
  }
];
