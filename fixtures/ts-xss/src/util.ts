// F01 regression fixture: the second payload probes the script-closer
// boundary — it must stay literal text in exports too.
import { run } from "./app"; // </script><img src=x onerror=alert(1)>

export function helper(input: string): string {
  if (input.length === 0) {
    return "helper";
  }
  return run(input.slice(1));
}
