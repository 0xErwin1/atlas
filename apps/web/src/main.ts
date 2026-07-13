import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from './App.vue';
import { expireSession } from './api/sessionExpiry';
import { setUnauthorizedHandler } from './api/wrapper';
import { router } from './router/index';
import { useAuthStore } from './stores/auth';
import './theme/index.css';

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);
app.use(router);

setUnauthorizedHandler(() => {
  const auth = useAuthStore(pinia);
  const redirect = expireSession(auth, router.currentRoute.value);

  if (redirect !== null) void router.replace(redirect);
});

app.mount('#app');
