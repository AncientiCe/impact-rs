import { fetchLater } from './lib'

export function viaBarrel() {
  return fetchLater(2)
}
