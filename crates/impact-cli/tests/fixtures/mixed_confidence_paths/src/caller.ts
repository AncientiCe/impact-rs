import { exact } from './target';

export function both(service: any) {
  exact();
  service.ambiguous();
}
