package b

type Printer struct{}

// Prune is this package's own, unrelated to a.Prune.
func (p Printer) Prune(x int) int {
	return x
}

func (p Printer) Run(x int) int {
	return p.Prune(x)
}
