package main

func registerRoutes(mux *http.ServeMux) {
	mux.HandleFunc("POST /payments", createPaymentRoute)
	mux.HandleFunc("/legacy", legacyHandler)
}
