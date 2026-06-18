import { bootstrapApplication } from '@angular/platform-browser';
import { provideAnimationsAsync } from '@angular/platform-browser/animations/async';
import { provideHttpClient } from '@angular/common/http';
import { provideRouter, withPreloading, PreloadAllModules } from '@angular/router';
import { AppComponent } from './app/app.component';
import { routes } from './app/app.routes';

bootstrapApplication(AppComponent, {
  providers: [
    // Lazy-load the animations engine — lighter first paint.
    provideAnimationsAsync(),
    provideHttpClient(),
    // Preload all lazy route chunks in the background after the first paint, so
    // navigating between pages is instant instead of fetching a chunk on click.
    provideRouter(routes, withPreloading(PreloadAllModules)),
  ]
}).catch(err => console.error(err));
