const U32_MODULUS = 2 ** 32

export function deltaOf(now: number, before: number): number {
  return now >= before ? now - before : now + U32_MODULUS - before
}
