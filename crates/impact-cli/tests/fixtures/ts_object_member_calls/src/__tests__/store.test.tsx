import { store } from '../store';

describe('store', () => {
  it('reads the stored value', async () => {
    expect(await store.get()).toBe('value');
  });
});
