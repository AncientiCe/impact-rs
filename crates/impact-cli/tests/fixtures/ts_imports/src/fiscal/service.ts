// Never imports store/sync/utils. Its own `prune` is a method on this class, and
// `this.prune(...)` used to resolve to whichever `prune` the linker found first.
export class Fiscal {
  prune(document) {
    return document;
  }

  send(document) {
    return this.prune(document);
  }
}
