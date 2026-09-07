import { camelizeOrder, prune } from './utils';

export function syncCart(order) {
  return prune(camelizeOrder(order));
}
