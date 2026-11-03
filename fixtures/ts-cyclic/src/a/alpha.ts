import { beta } from "../b/beta";

export function alpha(input: string): string {
  return `alpha(${beta(input)})`;
}
