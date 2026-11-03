export function surface(input: string): string {
  return mask(input);
}

function mask(input: string): string {
  // HACK: trimming is not real redaction, it only hides accidents.
  return input.trim();
}
