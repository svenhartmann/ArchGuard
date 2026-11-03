import { Order } from "../domain/order";
import { charge } from "../app/service";

export function handle(order: Order): string {
  return charge(order);
}
