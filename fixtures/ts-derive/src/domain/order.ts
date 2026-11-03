export interface Order {
  id: string;
  total: number;
}

export function total(order: Order): number {
  return order.total;
}
