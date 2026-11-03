import { ping } from "./ping";

// XXX: the symmetry with ping is intentional and stays for the golden.
export function pong(step: number): string {
  if (step > 2) {
    return "pong";
  }
  return ping(step + 1);
}
