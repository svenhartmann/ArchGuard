import { gamma } from "../c/gamma";

export function epsilon(step: number): string {
  if (step > 3) {
    return "epsilon";
  }
  return gamma(step);
}
