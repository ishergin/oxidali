import { createContext } from 'preact'
import { useCallback, useContext, useEffect, useState } from 'preact/hooks'
import type { ComponentChildren } from 'preact'

import { api } from './api/client'

const REFRESH_MS = 30_000

const VerifyingContext = createContext<boolean>(true)

export function ObservationProvider({ children }: { children: ComponentChildren }) {
    const [verifying, setVerifying] = useState(true)

    const read = useCallback(() => {
        api
            .pollerSettings()
            .then((s) => setVerifying(s.enabled))
            .catch(() => undefined)
    }, [])

    useEffect(() => {
        read()
        const timer = setInterval(read, REFRESH_MS)
        const onChanged = () => read()
        window.addEventListener(POLLER_SETTINGS_CHANGED, onChanged)
        return () => {
            clearInterval(timer)
            window.removeEventListener(POLLER_SETTINGS_CHANGED, onChanged)
        }
    }, [read])

    return <VerifyingContext.Provider value={verifying}>{children}</VerifyingContext.Provider>
}

export function useVerifying(): boolean {
    return useContext(VerifyingContext)
}

export const POLLER_SETTINGS_CHANGED = 'dali2rust:poller-settings-changed'

export function notifyPollerSettingsChanged(): void {
    window.dispatchEvent(new Event(POLLER_SETTINGS_CHANGED))
}
