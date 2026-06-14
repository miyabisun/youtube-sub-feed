import { getBasePath } from '$lib/router.svelte.js'

export default {
  path: {
    get api() {
      return `${getBasePath()}/api`
    },
  },
  // Google Identity Services client ID (public, safe to embed in JS).
  // Set VITE_GIS_CLIENT_ID at build time (or at runtime via window.__GIS_CLIENT_ID__).
  get gisClientId() {
    return (
      (typeof window !== 'undefined' && window.__GIS_CLIENT_ID__) ||
      import.meta.env.VITE_GIS_CLIENT_ID ||
      ''
    )
  },
}
