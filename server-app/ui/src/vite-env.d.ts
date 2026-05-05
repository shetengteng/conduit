/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}

// Vite define-injected build-time constant. See vite.config.ts.
declare const __APP_VERSION__: string;
