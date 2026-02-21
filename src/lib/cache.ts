interface CacheEntry {
  value: unknown
  expiresAt: number | null
}

const MAX_ENTRIES = 10000
const SWEEP_INTERVAL = 60 * 60 * 1000 // 1 hour

const store = new Map<string, CacheEntry>()

function sweep() {
  const now = Date.now()
  for (const [key, entry] of store) {
    if (entry.expiresAt && now > entry.expiresAt) {
      store.delete(key)
    }
  }
}

setInterval(sweep, SWEEP_INTERVAL).unref()

export default {
  get(key: string): unknown | null {
    const entry = store.get(key)
    if (!entry) return null
    if (entry.expiresAt && Date.now() > entry.expiresAt) {
      store.delete(key)
      return null
    }
    return entry.value
  },

  set(key: string, value: unknown, ttlSeconds?: number): void {
    if (store.size >= MAX_ENTRIES && !store.has(key)) {
      const oldest = store.keys().next().value!
      store.delete(oldest)
    }
    store.set(key, {
      value,
      expiresAt: ttlSeconds ? Date.now() + ttlSeconds * 1000 : null,
    })
  },

  del(key: string): void {
    store.delete(key)
  },

  clear(): void {
    store.clear()
  },

  clearPrefix(prefix: string): void {
    for (const key of store.keys()) {
      if (key.startsWith(prefix)) store.delete(key)
    }
  },
}
