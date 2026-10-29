import type { Operation, OperationAccepted } from './api/types.js'
import { isPreempted } from './format.js'

export interface RetryPorts {
  start: () => Promise<OperationAccepted>
  track: (accepted: OperationAccepted, willRetry: boolean) => Promise<Operation | null>
  startFailed: (e: unknown, retried: boolean) => void
  pause: () => Promise<void>
}

export async function runWithRetries(ports: RetryPorts, retries: number): Promise<Operation | null> {
  for (let attempt = 0; ; attempt++) {
    let accepted: OperationAccepted
    try {
      accepted = await ports.start()
    } catch (e) {
      ports.startFailed(e, attempt > 0)
      return null
    }
    const op = await ports.track(accepted, attempt < retries)
    if (!isPreempted(op) || attempt >= retries) return op
    await ports.pause()
  }
}
