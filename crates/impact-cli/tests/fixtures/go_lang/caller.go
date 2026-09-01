package main

type Consumer struct{}

func (c Consumer) Run() bool {
	return Process()
}
