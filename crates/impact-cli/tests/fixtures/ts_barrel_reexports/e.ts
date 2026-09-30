import { print } from './cycle/a'

export function viaCycle() {
  return print('e')
}
