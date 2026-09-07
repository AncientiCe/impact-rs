package b

import "example.com/gochained/a"

func Run() bool {
	return a.BuildClient().Send()
}
