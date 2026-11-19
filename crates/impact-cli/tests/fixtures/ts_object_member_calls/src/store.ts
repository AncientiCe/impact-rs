import type { Storage } from 'some-storage-lib';

export const store: Storage<string> = {
  get: async () => 'value',
  set: async (value: string) => {
    void value;
  },
  update: (apply: (current: string) => string) => apply('value'),
};
