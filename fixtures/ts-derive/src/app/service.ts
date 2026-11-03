import { Order, total } from "../domain/order";

export function charge(order: Order): string {
  return `charged:${total(order)}`;
}
