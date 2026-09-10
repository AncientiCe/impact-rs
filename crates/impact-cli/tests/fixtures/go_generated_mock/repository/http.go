package repository

type HTTPRepository struct {
	baseURL string
}

func (c *HTTPRepository) Notify(ctx string) error {
	return nil
}
