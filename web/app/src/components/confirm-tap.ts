export interface Tap<K> {
  armed: K | null
  act: boolean
}

export function nextTap<K>(armed: K | null, key: K): Tap<K> {
  return armed === key ? { armed: null, act: true } : { armed: key, act: false }
}
