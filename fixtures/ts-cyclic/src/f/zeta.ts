import { eta } from "../g/eta";

export function zeta(items: string[]): string[] {
  return items.map((item) => eta(item));
}
