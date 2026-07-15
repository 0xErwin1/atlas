import { defineStore } from 'pinia';
import { reactive } from 'vue';

export type ResourceStatus =
  | 'empty'
  | 'ready'
  | 'refreshing'
  | 'reconnecting'
  | 'offline'
  | 'error-with-data'
  | 'error-empty';

interface ResourceState {
  status: ResourceStatus;
  usable: boolean;
}

export const useResourceStatusStore = defineStore('resource-status', () => {
  const resources = reactive<Record<string, ResourceState>>({});

  function stateFor(key: string): ResourceState {
    return resources[key] ?? { status: 'empty', usable: false };
  }

  function setReady(key: string, usable: boolean): void {
    resources[key] = { status: usable ? 'ready' : 'empty', usable };
  }

  function setRefreshing(key: string): void {
    const current = stateFor(key);
    resources[key] = {
      status: current.usable && current.status !== 'error-with-data' ? 'refreshing' : current.status,
      usable: current.usable,
    };
  }

  function beginRequest(key: string, onlineHint: boolean): void {
    const current = stateFor(key);
    if (!current.usable) return;

    resources[key] = {
      status: onlineHint ? 'refreshing' : 'reconnecting',
      usable: true,
    };
  }

  function recordRequestSuccess(key: string, usable: boolean): void {
    setReady(key, usable);
  }

  function recordRequestFailure(key: string, onlineHint: boolean): void {
    const current = stateFor(key);
    resources[key] = {
      status: current.usable && !onlineHint ? 'offline' : current.usable ? 'error-with-data' : 'error-empty',
      usable: current.usable,
    };
  }

  function setReconnecting(key: string): void {
    const current = stateFor(key);
    resources[key] = { status: current.usable ? 'reconnecting' : 'empty', usable: current.usable };
  }

  function setOffline(key: string): void {
    const current = stateFor(key);
    resources[key] = { status: current.usable ? 'offline' : 'error-empty', usable: current.usable };
  }

  function setError(key: string, usable: boolean): void {
    resources[key] = { status: usable ? 'error-with-data' : 'error-empty', usable };
  }

  function statusFor(key: string): ResourceStatus {
    return stateFor(key).status;
  }

  function usesFullLoader(key: string): boolean {
    return statusFor(key) === 'empty';
  }

  function clear(key: string): void {
    delete resources[key];
  }

  return {
    beginRequest,
    clear,
    recordRequestFailure,
    recordRequestSuccess,
    setError,
    setOffline,
    setReady,
    setReconnecting,
    setRefreshing,
    statusFor,
    usesFullLoader,
  };
});
