export interface Greeter {
  greet(name: string): string
  farewell: (name: string) => string
}

const impl: Greeter = {
  greet: (name) => `Hello, ${name}`,
  farewell: (name) => `Goodbye, ${name}`,
}

export default impl
