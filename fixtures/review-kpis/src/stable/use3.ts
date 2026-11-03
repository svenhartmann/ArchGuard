import { store } from "./store";

export function useThree(input: string): string {
  return store(`three:${input}`);
}
