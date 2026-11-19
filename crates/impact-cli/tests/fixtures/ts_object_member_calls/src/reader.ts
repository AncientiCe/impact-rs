import { store } from './store';

export async function readValue() {
  return store.get();
}
