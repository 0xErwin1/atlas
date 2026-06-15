import { createRouter, createWebHistory } from 'vue-router';
import { routes } from './routes';

export const router = createRouter({
  history: createWebHistory(),
  routes,
});

router.beforeEach(async (to, _from) => {
  if (to.name === 'login') return true;

  const { useAuthStore } = await import('@/stores/auth');
  const auth = useAuthStore();

  if (!auth.isAuthenticated) {
    await auth.fetchMe();
  }

  if (!auth.isAuthenticated) {
    return { name: 'login', query: { redirect: to.fullPath } };
  }

  return true;
});
