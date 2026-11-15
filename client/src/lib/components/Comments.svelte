<script module>
  import config from '$lib/config.js'
  import { createTokenCache, requestGisToken } from '$lib/gis.js'
  import { COMMENT_SCOPE } from '$lib/comments.js'

  // Kept for the visit, per video: closing the player by accident loses neither
  // a half-written comment nor the handle on the ones already posted.
  const drafts = new Map()
  const posted = new Map()
  const tokens = createTokenCache(() => requestGisToken(config.gisClientId, COMMENT_SCOPE))
</script>

<script>
  import { untrack } from 'svelte'
  import fetcher from '$lib/fetcher.js'
  import { relativeTime } from '$lib/relative-time.js'
  import {
    COMMENT_MAX_LENGTH,
    canSubmit,
    commentErrorMessage,
    deleteCommentRequest,
    insertCommentRequest,
    postedComment,
    sendComment,
    updateCommentRequest,
  } from '$lib/comments.js'
  import Icon from '$lib/components/Icon.svelte'
  import ClampedText from '$lib/components/ClampedText.svelte'
  import Spinner from '$lib/components/Spinner.svelte'

  // One instance per video: the parent re-creates this component when the video changes.
  let { videoId } = $props()

  let text = $state(untrack(() => drafts.get(videoId) ?? ''))
  let mine = $state(untrack(() => posted.get(videoId) ?? []))
  let sending = $state(false)
  let sendError = $state(null)

  let status = $state('loading')
  let items = $state([])
  let nextPage = $state(null)
  let loadingMore = $state(false)
  let moreFailed = $state(false)

  let editing = $state(null)
  let confirmingDelete = $state(null)
  let busy = $state(false)
  let itemError = $state(null)

  let canPost = $derived(!!config.gisClientId)
  let others = $derived(items.filter((item) => !mine.some((own) => own.id === item.id)))

  $effect(() => {
    drafts.set(videoId, text)
  })
  $effect(() => {
    posted.set(videoId, mine)
  })

  async function load(pageToken = null) {
    if (pageToken) loadingMore = true
    else status = 'loading'
    moreFailed = false
    try {
      let url = `${config.path.api}/videos/${videoId}/comments`
      if (pageToken) url += `?page_token=${encodeURIComponent(pageToken)}`
      const data = await fetcher(url)
      if (pageToken && data.status !== 'ok') throw new Error(data.status)
      items = pageToken ? [...items, ...data.items] : data.items
      nextPage = data.next_page_token
      status = data.status
    } catch {
      if (pageToken) moreFailed = true
      else status = 'error'
    } finally {
      loadingMore = false
    }
  }

  load()

  async function submit(event) {
    event?.preventDefault()
    if (!canSubmit(text) || sending) return
    sending = true
    sendError = null
    try {
      const thread = await sendComment(insertCommentRequest(videoId, text), { tokens })
      mine = [postedComment(thread), ...mine]
      text = ''
    } catch (error) {
      sendError = commentErrorMessage(error)
    } finally {
      sending = false
    }
  }

  function submitOnShortcut(event) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) submit(event)
  }

  function startEdit(comment) {
    itemError = null
    confirmingDelete = null
    editing = { id: comment.id, text: comment.text }
  }

  async function saveEdit(event) {
    event.preventDefault()
    if (!canSubmit(editing.text) || busy) return
    busy = true
    itemError = null
    try {
      const updated = await sendComment(updateCommentRequest(editing.id, editing.text), { tokens })
      const next = updated ? postedComment(updated).text : editing.text.trim()
      mine = mine.map((own) => (own.id === editing.id ? { ...own, text: next } : own))
      editing = null
    } catch (error) {
      itemError = { id: editing.id, message: commentErrorMessage(error) }
    } finally {
      busy = false
    }
  }

  async function remove(id) {
    if (busy) return
    busy = true
    itemError = null
    try {
      await sendComment(deleteCommentRequest(id), { tokens })
      mine = mine.filter((own) => own.id !== id)
      confirmingDelete = null
    } catch (error) {
      itemError = { id, message: commentErrorMessage(error) }
    } finally {
      busy = false
    }
  }
</script>

{#snippet meta(comment)}
  {#if comment.author_avatar}
    <img class="avatar" src={comment.author_avatar} alt="" loading="lazy" />
  {/if}
  <span class="author">{comment.author}</span>
  <span class="time">{relativeTime(comment.published_at)}</span>
{/snippet}

<section class="comments" aria-label="コメント">
  {#if status === 'disabled'}
    <p class="note">この動画はコメントを受け付けていません</p>
  {:else}
    {#if canPost}
      <form class="compose" onsubmit={submit}>
        <textarea
          bind:value={text}
          onkeydown={submitOnShortcut}
          placeholder="コメントを書く"
          aria-label="コメント"
          rows="3"></textarea>
        {#if sendError}
          <p class="error" role="alert">{sendError}</p>
        {/if}
        <div class="row">
          {#if text.trim().length > COMMENT_MAX_LENGTH}
            <span class="error">{text.trim().length} / {COMMENT_MAX_LENGTH}</span>
          {/if}
          <button class="primary" type="submit" disabled={!canSubmit(text) || sending}>
            {sending ? '送信中' : '送信'}
          </button>
        </div>
      </form>
    {:else}
      <p class="note">GIS_CLIENT_ID が未設定のため、コメントは送れません</p>
    {/if}

    <ul class="list">
      {#each mine as comment (comment.id)}
        <li class="comment own">
          <div class="meta">
            {@render meta(comment)}
            {#if confirmingDelete === comment.id}
              <span class="actions">
                <button class="danger" disabled={busy} onclick={() => remove(comment.id)}
                  >削除確認</button
                >
                <button class="quiet" onclick={() => (confirmingDelete = null)}>キャンセル</button>
              </span>
            {:else if editing?.id !== comment.id}
              <span class="actions">
                <button class="icon-btn" aria-label="編集" onclick={() => startEdit(comment)}>
                  <Icon>
                    <path d="M12 20h9" />
                    <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
                  </Icon>
                </button>
                <button
                  class="icon-btn"
                  aria-label="削除"
                  onclick={() => {
                    itemError = null
                    confirmingDelete = comment.id
                  }}
                >
                  <Icon>
                    <polyline points="3 6 5 6 21 6" />
                    <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                    <path d="M10 11v6M14 11v6" />
                    <path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
                  </Icon>
                </button>
              </span>
            {/if}
          </div>
          {#if editing?.id === comment.id}
            <form class="compose" onsubmit={saveEdit}>
              <textarea bind:value={editing.text} aria-label="コメントを編集" rows="3"></textarea>
              <div class="row">
                <button class="quiet" type="button" onclick={() => (editing = null)}
                  >キャンセル</button
                >
                <button class="primary" type="submit" disabled={!canSubmit(editing.text) || busy}
                  >保存</button
                >
              </div>
            </form>
          {:else}
            <ClampedText text={comment.text} />
          {/if}
          {#if itemError?.id === comment.id}
            <p class="error" role="alert">{itemError.message}</p>
          {/if}
        </li>
      {/each}

      {#each others as comment (comment.id)}
        <li class="comment">
          <div class="meta">
            {@render meta(comment)}
            {#if comment.like_count > 0}
              <span class="likes" aria-label={`高評価 ${comment.like_count}`}>
                <Icon>
                  <path
                    d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"
                  />
                </Icon>
                {comment.like_count}
              </span>
            {/if}
          </div>
          <ClampedText text={comment.text} />
        </li>
      {/each}
    </ul>

    {#if status === 'loading'}
      <Spinner />
    {:else if status === 'error'}
      <p class="error">
        コメントを読み込めませんでした
        <button class="quiet" onclick={() => load()}>再試行</button>
      </p>
    {:else if status === 'quota'}
      <p class="note">API の割り当て切れのため、今日はコメントを読み込めません</p>
    {:else if status === 'ok' && items.length === 0 && mine.length === 0}
      <p class="note">コメントはまだありません</p>
    {:else if nextPage}
      {#if moreFailed}
        <p class="error">読み込めませんでした</p>
      {/if}
      <button class="more" disabled={loadingMore} onclick={() => load(nextPage)}>
        {loadingMore ? '読み込み中' : 'もっと読む'}
      </button>
    {/if}
  {/if}
</section>

<style lang="sass">
.comments
	display: flex
	flex-direction: column
	gap: var(--sp-3)

.compose
	display: flex
	flex-direction: column
	gap: var(--sp-2)

textarea
	width: 100%
	min-height: 5em
	max-height: 50dvh
	padding: var(--sp-2) var(--sp-3)
	background: var(--c-surface)
	color: var(--c-text)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font: inherit
	line-height: 1.6
	resize: vertical
	// Grow with the text instead of scrolling inside a two-line box.
	field-sizing: content

	&:focus
		border-color: var(--c-accent)

.row
	display: flex
	align-items: center
	justify-content: flex-end
	gap: var(--sp-2)

.primary, .quiet, .danger, .more
	padding: var(--sp-2) var(--sp-4)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-sm)
	cursor: pointer

	&:disabled
		opacity: 0.5
		cursor: default

.primary
	background: var(--c-accent)
	color: var(--c-on-accent)
	border-color: var(--c-accent)

.quiet, .more
	background: transparent
	color: var(--c-text-sub)

	&:hover:not(:disabled)
		background: var(--c-overlay-2)
		color: var(--c-text)

.danger
	background: var(--c-danger-bg)
	color: var(--c-danger)
	border-color: var(--c-danger-border)

.more
	align-self: center

.note, .error
	margin: 0
	font-size: var(--fs-sm)

.note
	color: var(--c-text-muted)

.error
	color: var(--c-danger)

.list
	display: flex
	flex-direction: column
	gap: var(--sp-4)

.comment
	display: flex
	flex-direction: column
	gap: var(--sp-1)
	min-width: 0

.own
	padding: var(--sp-3)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)

.meta
	display: flex
	align-items: center
	gap: var(--sp-2)
	min-height: 24px
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	min-width: 0

.avatar
	width: 24px
	height: 24px
	border-radius: var(--radius-full)
	flex-shrink: 0

.author
	overflow: hidden
	text-overflow: ellipsis
	white-space: nowrap

.time, .likes
	color: var(--c-text-muted)
	flex-shrink: 0

.likes
	display: inline-flex
	align-items: center
	gap: var(--sp-1)

.actions
	display: inline-flex
	align-items: center
	gap: var(--sp-1)
	margin-left: auto
	flex-shrink: 0

	.danger, .quiet
		padding: var(--sp-1) var(--sp-3)
		font-size: var(--fs-xs)

.icon-btn
	display: inline-flex
	align-items: center
	justify-content: center
	width: 32px
	height: 32px
	background: transparent
	color: var(--c-text-sub)
	border: 0
	border-radius: var(--radius-sm)
	font-size: var(--fs-lg)
	cursor: pointer

	&:hover
		background: var(--c-overlay-2)
		color: var(--c-text)
</style>
