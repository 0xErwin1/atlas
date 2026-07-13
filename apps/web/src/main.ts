import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from './App.vue';
import { expireSession } from './api/sessionExpiry';
import { setUnauthorizedHandler } from './api/wrapper';
import { disposeWorkspaceLiveUpdates } from './lib/workspaceLiveUpdates';
import { router } from './router/index';
import { useAuthStore } from './stores/auth';
import './theme/index.css';

const app = createApp(App);
const pinia = createPinia();
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

app.use(pinia);
app.use(router);

setUnauthorizedHandler(() => {
  const auth = useAuthStore(pinia);
  const redirect = expireSession(auth, router.currentRoute.value);

  if (redirect !== null) void router.replace(redirect);
});

registerWorkspaceLiveUpdatesPagehide();
app.mount('#app');
