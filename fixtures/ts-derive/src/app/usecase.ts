import { Customer, displayName } from "../domain/customer";

export function greet(customer: Customer): string {
  return `hello ${displayName(customer)}`;
}
