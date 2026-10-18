// Which kinds the feed shows. Session state: every launch starts with both on,
// and it survives the Feed remount that group switches and swipes cause.
export const feedKinds = $state({ video: true, post: true })

export function kindsParam(kinds) {
  return ['video', 'post'].filter((kind) => kinds[kind]).join(',')
}
