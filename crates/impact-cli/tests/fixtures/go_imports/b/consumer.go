package b

import "example.com/goimports/a"

func Consume(x int) int {
	return a.Prune(x)
}
