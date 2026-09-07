import * as helpers from './store/sync/utils';

export function Widget(props) {
  return helpers.camelizeOrder(props);
}
