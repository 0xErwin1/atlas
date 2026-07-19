/// <reference types="vite/client" />

// Build-time constants injected by Vite `define` (see vite.config.ts).
declare const __ATLAS_BUILD_VERSION__: string;
declare const __ATLAS_BUILD_COMMIT__: string;

declare module '*.vue' {
  import type { DefineComponent } from 'vue';
  const component: DefineComponent;
  export default component;
}
