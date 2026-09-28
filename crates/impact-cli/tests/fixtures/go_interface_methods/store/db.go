package store

// DB is the only real implementation of Store.
type DB struct{}

func (d *DB) Fetch(id string) (string, error) {
	return id, nil
}
