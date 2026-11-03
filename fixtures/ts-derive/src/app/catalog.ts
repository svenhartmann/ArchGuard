import { Product, priceOf } from "../domain/product";

export function format(product: Product): string {
  return `${product.sku}@${priceOf(product)}`;
}
