// Fires requests through a callback; never touches bindAll or patch.
export function fire(handler, args) {
  return handler.apply(null, args);
}

export function fireLater(handler, args) {
  return handler.call(null, ...args);
}

export function headerNames(headers) {
  return Reflect.ownKeys(headers);
}
