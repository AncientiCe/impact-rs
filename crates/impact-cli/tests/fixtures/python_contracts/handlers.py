from repo import save_payment


@app.get("/payments")
def create_payment_route():
    return save_payment()


@app.route("/orders", methods=["POST"])
def create_order_route():
    return save_payment()


def legacy_handler():
    return True
