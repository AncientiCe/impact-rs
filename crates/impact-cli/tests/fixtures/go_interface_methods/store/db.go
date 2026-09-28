package store

// DB is the only real implementation of Store; its Save lives in db_write.go.
type DB struct{}

func (d *DB) Fetch(id string) (string, error) {
	return id, nil
}
