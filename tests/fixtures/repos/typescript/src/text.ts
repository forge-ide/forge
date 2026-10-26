/** `name` with its first letter capitalized. */
export function title(name: string): string {
  return name.charAt(0).toUpperCase() + name.slice(1);
}
