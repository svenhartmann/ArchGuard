import { alpha } from "../a/alpha";

export function beta(input: string): string {
  if (input.length === 0) {
    return "beta";
  }
  return alpha(input.slice(1));
}
