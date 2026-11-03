// F01 regression fixture: the HTML payload in this import comment must
// appear ONLY as literal text in viewer tooltips — never as an element.
import { helper } from "./util"; // <img src=x onerror=alert(1)>

export function run(input: string): string {
  return helper(`run(${input})`);
}
