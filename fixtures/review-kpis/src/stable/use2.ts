import { store } from "./store";

export function useTwo(input: string): string {
  return store(`two:${input}`);
}
