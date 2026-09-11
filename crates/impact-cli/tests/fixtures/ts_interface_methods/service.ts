export interface Greeter {
  greet(name: string): string
}

const impl: Greeter = {
  greet: (name) => `Hello, ${name}`,
}

export default impl
