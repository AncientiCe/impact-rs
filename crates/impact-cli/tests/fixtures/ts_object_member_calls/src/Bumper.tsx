import { store } from './store';

export function bump() {
  store.update((current) => current + '1');
}
