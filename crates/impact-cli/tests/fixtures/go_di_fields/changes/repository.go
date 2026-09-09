package changes

// Repository is the interface a use-case depends on, idiomatic Go dependency injection —
// the use-case struct holds a field of this interface type rather than the concrete type.
type Repository interface {
	AddOperation(op string) bool
}

// repositoryImpl is the only real implementation in this fixture, so its AddOperation is
// the one symbol a caller through the interface's field type actually resolves against.
type repositoryImpl struct{}

func (r *repositoryImpl) AddOperation(op string) bool {
	return true
}
