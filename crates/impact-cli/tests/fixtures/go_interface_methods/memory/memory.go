package memory

// Cache implements store.Store from another package, the way an adapter implements a
// port: nothing in its own source names the interface.
type Cache struct {
	values map[string]string
}

func (c *Cache) Fetch(id string) (string, error) {
	return c.values[id], nil
}

func (c *Cache) Save(id string, value string) error {
	c.values[id] = value
	return nil
}

// Lookup has a Fetch and a Save too, but with other signatures, so it isn't a Store.
type Lookup struct{}

func (l Lookup) Fetch(id int) string {
	return ""
}

func (l Lookup) Save(id int) error {
	return nil
}

// Closer has ReadStore's own Close, but not the Store methods ReadStore embeds.
type Closer struct{}

func (c Closer) Close() error {
	return nil
}
