package usecase

import "example.com/godi/changes"

// UseCase is the dominant Go DI idiom this fixture reproduces: a struct field typed as an
// interface, populated by whatever wires up the use-case, called through a one-hop
// selector chain (`uc.ChangesRepository.AddOperation(...)`) rather than a plain
// `receiver.Method()` call.
type UseCase struct {
	ChangesRepository changes.Repository
}

func (uc *UseCase) Do() bool {
	return uc.ChangesRepository.AddOperation("op")
}
