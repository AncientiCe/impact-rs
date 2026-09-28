package service

import "example.com/app/store"

// Load reaches Fetch only through a parameter typed as the interface.
func Load(s store.Store, id string) string {
	v, _ := s.Fetch(id)
	return v
}
