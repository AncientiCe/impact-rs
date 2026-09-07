class Client:
    """Never imports a.util. `camelize_order` here is its own method."""

    def camelize_order(self, order):
        return order

    def run(self, order):
        return self.camelize_order(order)
