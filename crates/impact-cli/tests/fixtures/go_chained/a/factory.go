package a

// The only thing this file declares, called as the receiver of a chained call.
func BuildClient() *Client {
	return &Client{}
}
