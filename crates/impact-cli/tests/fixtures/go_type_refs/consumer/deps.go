package consumer

import "example.com/gotyperefs/source"

// Dependencies is the dependency-injection shape: an imported struct type held by a field.
type Dependencies struct {
	Svc source.Service
}
