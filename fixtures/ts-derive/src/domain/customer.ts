export interface Customer {
  id: string;
  name: string;
}

export function displayName(customer: Customer): string {
  return customer.name.trim();
}
