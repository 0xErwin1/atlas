import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from './App.vue';
import { disposeWorkspaceLiveUpdates } from './lib/workspaceLiveUpdates';
import { router } from './router/index';
import './theme/index.css';

const app = createApp(App);
let removePagehideListener: (() => void) | null = null;

export function registerWorkspaceLiveUpdatesPagehide(): () => void {
  if (removePagehideListener !== null) return removePagehideListener;

  const onPagehide = (event: PageTransitionEvent) => {
    if (!event.persisted) disposeWorkspaceLiveUpdates();
  };

  window.addEventListener('pagehide', onPagehide);
  removePagehideListener = () => {
    window.removeEventListener('pagehide', onPagehide);
    removePagehideListener = null;
  };
  return removePagehideListener;
}

app.use(createPinia());
app.use(router);
registerWorkspaceLiveUpdatesPagehide();
app.mount('#app');
