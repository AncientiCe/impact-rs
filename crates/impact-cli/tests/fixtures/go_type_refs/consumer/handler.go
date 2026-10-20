package consumer

import "example.com/gotyperefs/source"

// Store names the type only in an interface method's signature.
type Store interface {
	Save(svc *source.Service) error
}

// Handle names the type only as a parameter.
func Handle(svc *source.Service) bool {
	return svc != nil
}

// Build names the type only as its result.
func Build() *source.Service {
	return nil
}

// Default names the type only in a composite literal.
func Default() string {
	svc := source.Service{Name: "default"}
	return svc.Name
}

// Unrelated names no type from source, so a rename of Service must not reach it.
func Unrelated() string {
	return "unrelated"
}
