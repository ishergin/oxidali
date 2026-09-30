import { useRef } from 'preact/hooks'
import { deltaOf, type Flat, flatten } from './counter-delta'

export { deltaOf, type Flat } from './counter-delta'

export function label(key: string): string {
  const words = key.replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

export function useDeltas<T>(
  data: T | null | undefined,
  sampleMs: number | null,
  view: (data: T) => unknown = (sample) => sample,
): Flat {
  const previous = useRef<Flat | null>(null)
  const sampledAt = useRef(-1)
  const deltas = useRef<Flat>({})

  if (data && sampleMs !== null && sampleMs !== sampledAt.current) {
    const flat = flatten(view(data), '', {})
    if (sampledAt.current >= 0 && sampleMs < sampledAt.current) {
      deltas.current = {}
    } else if (previous.current) {
      const next: Flat = {}
      for (const [path, value] of Object.entries(flat)) {
        const before = previous.current[path]
        if (before !== undefined) next[path] = deltaOf(value, before)
      }
      deltas.current = next
    }
    previous.current = flat
    sampledAt.current = sampleMs
  }
  return deltas.current
}

export function CounterRows({
  block,
  path,
  deltas,
  isFault,
  gauges,
}: {
  block: Record<string, number>
  path: string
  deltas: Flat
  isFault?: (key: string, value: number) => boolean
  gauges?: readonly string[]
}) {
  return (
    <>
      {Object.entries(block).map(([key, value]) => {
        const delta = gauges?.includes(key) ? 0 : deltas[path ? `${path}.${key}` : key] ?? 0
        const fault = isFault?.(key, value) ?? false
        return (
          <div class="attr" key={key}>
            <span class="k">{label(key)}</span>
            <span class={`v${fault ? ' fault' : ''}`}>{value.toLocaleString()}</span>
            <span class={`delta${delta > 0 ? ' live' : ''}`}>{delta > 0 ? `+${delta}` : ''}</span>
          </div>
        )
      })}
    </>
  )
}
