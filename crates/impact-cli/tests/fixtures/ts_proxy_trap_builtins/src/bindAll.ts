// Wraps an object so every method read off it stays bound to the original target.
export function bindAll<T extends object>(target: T): T {
  return new Proxy(target, {
    get(obj, prop, receiver) {
      const value = Reflect.get(obj, prop, receiver);
      if (typeof value === "function") {
        return (...args: unknown[]) => value.apply(obj, args);
      }
      return value;
    },
    apply(fn, thisArg, args) {
      return Reflect.apply(fn as Function, thisArg, args);
    },
  });
}
