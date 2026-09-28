package store

// Store is declared on its own and implemented in sibling files — the file a signature
// change to Fetch starts in.
type Store interface {
	Fetch(id string) (string, error)
	Save(id string, value string) error
}

// ReadStore embeds Store, so its full method set isn't visible from here.
type ReadStore interface {
	Store
	Close() error
}
