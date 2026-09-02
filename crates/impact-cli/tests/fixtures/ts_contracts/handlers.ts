import { savePayment } from "./repo";

export function createPaymentRoute(): boolean {
  return savePayment();
}
