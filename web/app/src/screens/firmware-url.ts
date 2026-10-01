export function urlProblem(url: string, maxBytes: number): string | null {
  if (!/^https?:\/\//.test(url)) return 'Must start with http:// or https://'
  const size = new TextEncoder().encode(url).length
  if (size > maxBytes) return `Too long: ${size} of ${maxBytes} bytes`
  return null
}
