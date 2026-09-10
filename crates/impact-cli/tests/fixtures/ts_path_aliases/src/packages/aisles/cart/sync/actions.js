import { camelizeOrder, prune } from '@newstore/aisles/cart/sync/utils';

export function syncCart(order) {
  return prune(camelizeOrder(order));
}
