package store

// Store is declared on its own and implemented in sibling files — the file a signature
// change to Fetch starts in.
type Store interface {
	Fetch(id string) (string, error)
}
