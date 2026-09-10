import { resetMocks } from '@testing/setup';
import { helper } from 'widget';

export function runAll() {
  resetMocks();
  return helper();
}
