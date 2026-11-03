import { epsilon } from "../e/epsilon";

export function delta(step: number): string {
  return `delta:${epsilon(step + 1)}`;
}
