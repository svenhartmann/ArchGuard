import { alpha } from "./a/alpha";
import { gamma } from "./c/gamma";
import { zeta } from "./f/zeta";

export function run(): string {
  return [alpha("x"), gamma(0), zeta([" A "])].join("|");
}
