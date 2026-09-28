package api

import "example.com/app/service"

func Handle(id string) string {
	return service.Load(nil, id)
}
