import { Database } from 'bun:sqlite'

const dbPath = process.env.DATABASE_PATH || './feed.db'

const sqlite = new Database(dbPath)
sqlite.exec('PRAGMA journal_mode = WAL')
sqlite.exec('PRAGMA synchronous = NORMAL')
sqlite.exec('PRAGMA cache_size = -64000')
sqlite.exec('PRAGMA temp_store = MEMORY')

export { sqlite }
