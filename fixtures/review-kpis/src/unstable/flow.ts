import { leafOne } from "./leaf1";
import { leafTwo } from "./leaf2";

// FIXME: this module is intentionally unstable; it fans out to both leaves.
export function flow(input: string): string {
  return `flow(${leafOne(input)}|${leafTwo(input)})`;
}
