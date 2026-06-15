import type { RouteRecordRaw } from 'vue-router';

export const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/views/Login.vue'),
  },
  {
    path: '/n/:slug?',
    name: 'notes',
    components: {
      default: () => import('@/views/Notes.vue'),
      sidebar: () => import('@/views/NotesSidebar.vue'),
    },
  },
  {
    path: '/t/:boardId',
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
