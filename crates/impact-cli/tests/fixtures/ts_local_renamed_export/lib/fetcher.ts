const inner = (opts: number) => opts + 1

const scheduled = (opts: number) => setTimeout(() => inner(opts), 10)

export { scheduled as inner }
