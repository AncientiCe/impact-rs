package usecase

import (
	"example.com/svc/repository"
)

type UseCase struct {
	Repo repository.Repository
}

func (uc UseCase) Do() error {
	return uc.Repo.Notify("x")
}
