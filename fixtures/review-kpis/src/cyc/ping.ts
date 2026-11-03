import { pong } from "./pong";

export function ping(step: number): string {
  if (step > 2) {
    return "ping";
  }
  return pong(step + 1);
}
