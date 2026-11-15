// The one write token of this visit. Comments and likes share it, so Google
// asks once however the viewer starts.
import config from '$lib/config.js'
import { createTokenCache, requestGisToken } from '$lib/gis.js'
import { WRITE_SCOPE } from '$lib/youtube-write.js'

export const tokens = createTokenCache(() => requestGisToken(config.gisClientId, WRITE_SCOPE))
