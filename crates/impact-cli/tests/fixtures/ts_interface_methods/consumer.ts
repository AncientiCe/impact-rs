import greeter from './service'

export function run(name: string): string {
  return greeter.greet(name)
}
