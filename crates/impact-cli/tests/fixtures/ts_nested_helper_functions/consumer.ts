import { doWork } from './target'

export function outer(): void {
  const helper = () => {
    doWork()
  }
  helper()
}
