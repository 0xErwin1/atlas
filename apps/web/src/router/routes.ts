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
    path: '/:pathMatch(.*)*',
    name: 'not-found',
    component: () => import('@/views/NotFound.vue'),
  },
];
