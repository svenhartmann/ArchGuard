import { delta } from "../d/delta";

export function gamma(step: number): string {
  return `gamma:${delta(step + 1)}`;
}
