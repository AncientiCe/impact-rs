import { camelizeOrder, prune } from '../utils';

describe('sync utils', () => {
  beforeEach(() => {
    prune({});
  });

  it('camelizes an order', () => {
    expect(camelizeOrder({})).toEqual({});
  });
});
