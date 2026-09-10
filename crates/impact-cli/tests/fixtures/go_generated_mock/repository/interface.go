package repository

//go:generate mockgen -source=interface.go -destination=interface_mock.go -package=repository

type Repository interface {
	Notify(ctx string) error
}
