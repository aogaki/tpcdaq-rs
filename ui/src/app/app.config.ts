import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    // ルートの `data`(プレースホルダの見出し・担当ユニット)を入力として受けるため。
    provideRouter(routes, withComponentInputBinding()),
  ],
};
