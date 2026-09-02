import { createPaymentRoute } from "./handlers";

app.get("/payments", createPaymentRoute);
app.get("/anonymous", (req, res) => {
  return true;
});
