import { flow } from "../unstable/flow";

// TODO: cache the derived rows instead of recomputing them per call.
export function store(input: string): string {
  return `store(${flow(input)})`;
}
