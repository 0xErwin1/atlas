import type { RouteRecordRaw } from 'vue-router';

export const routes: RouteRecordRaw[] = [
  {
    path: '/',
    redirect: '/n',
  },
  {
    path: '/login',
    name: 'login',
    component: () => import('@/views/Login.vue'),
  },
  {
    path: '/activate/:token',
    name: 'activate',
    component: () => import('@/views/ActivateView.vue'),
    meta: { public: true },
  },
  {
    path: '/n/:slug?',
    name: 'notes',
    component: () => import('@/views/Notes.vue'),
  },
  {
    path: '/t/views/:viewId',
    name: 'task-view',
    component: () => import('@/views/Tasks.vue'),
  },
  {
    path: '/t/:boardId?',
    name: 'tasks',
    component: () => import('@/views/Tasks.vue'),
    // Notes and Tasks share one rail entry now: a boardless tasks location — the
    // bare `/t` URL or a `{ name: 'tasks' }` navigation with no board — has no
    // standalone landing page, so it redirects into the unified `/n` entry. Deep
    // board links (`/t/:boardId`) still resolve to the board view.
    beforeEnter: (to) => {
      const boardId = to.params.boardId;
      const hasBoard = typeof boardId === 'string' && boardId.length > 0;
      return hasBoard ? true : { name: 'notes' };
    },
  },
  {
    path: '/t/task/:readableId',
    name: 'task-detail',
    component: () => import('@/views/TaskDetail.vue'),
  },
  {
    path: '/search',
    name: 'search',
    component: () => import('@/views/Search.vue'),
  },
  {
    path: '/settings/:section?',
    name: 'settings',
    component: () => import('@/views/SettingsView.vue'),
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'not-found',
    component: () => import('@/views/NotFound.vue'),
  },
];
