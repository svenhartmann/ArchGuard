import { Customer } from "../domain/customer";
import { greet } from "../app/usecase";

export function main(customer: Customer): string {
  return greet(customer);
}
