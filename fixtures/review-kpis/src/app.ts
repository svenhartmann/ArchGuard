import { store } from "./stable/store";
import { mask } from "./internal/hidden";

// FIXME: reaching into hidden's internals is deliberate for this fixture.
export function run(input: string): string {
  return `${store(input)} ${mask(input)}`;
}
