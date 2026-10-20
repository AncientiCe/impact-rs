package source

// Service is the type this fixture renames. Nothing calls it: every consumer only names
// it as a type, which is still a dependency a rename or move has to update.
type Service struct {
	Name string
}
