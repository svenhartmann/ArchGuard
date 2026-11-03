export interface Product {
  sku: string;
  price: number;
}

export function priceOf(product: Product): number {
  return product.price;
}
