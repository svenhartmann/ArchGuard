import { store } from "./store";

export function useOne(input: string): string {
  return store(`one:${input}`);
}
