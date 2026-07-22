<script>
  let { channel, onclose, ontoggle, toggling = false } = $props()
  let dialogElement = $state(null)
  let favoriteButton = $state(null)

  $effect(() => {
    const dialog = dialogElement
    if (!dialog) return
    dialog.showModal()
    queueMicrotask(() => favoriteButton?.focus())
    return () => {
      if (dialog.open) dialog.close()
    }
  })

  function trapFocus(event) {
    if (event.key !== 'Tab') return
    const buttons = [...dialogElement.querySelectorAll('button:not(:disabled)')]
    const first = buttons[0]
    const last = buttons.at(-1)
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }
</script>

<dialog
  class="channel-dialog"
  aria-labelledby="channel-menu-title"
  bind:this={dialogElement}
  oncancel={(event) => {
    event.preventDefault()
    onclose()
  }}
  onclick={(event) => event.target === event.currentTarget && onclose()}
  onkeydown={trapFocus}
>
  <div class="context-menu">
    <div class="menu-heading">
      <span class="eyebrow">チャンネル操作</span>
      <h2 id="channel-menu-title">{channel.title}</h2>
    </div>
    <button
      class="favorite-action"
      class:active={channel.is_favorite}
      onclick={ontoggle}
      disabled={toggling}
      bind:this={favoriteButton}
    >
      <span class="star" aria-hidden="true">★</span>
      <span
        >{toggling
          ? '更新中…'
          : channel.is_favorite
            ? 'お気に入りから外す'
            : 'お気に入りに追加'}</span
      >
    </button>
    <button class="cancel-action" onclick={onclose}>キャンセル</button>
  </div>
</dialog>

<style lang="sass">
dialog.channel-dialog
	width: min(calc(100vw - 32px), 360px)
	max-width: none
	padding: 0
	color: inherit
	background: transparent
	border: 0
	overflow: visible

	&::backdrop
		background: var(--c-scrim)

.context-menu
	width: 100%
	padding: var(--sp-4)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-lg)
	box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25)
	max-height: 80dvh
	overflow-y: auto

.menu-heading
	padding: 0 var(--sp-1) var(--sp-3)
	border-bottom: 1px solid var(--c-border)

.eyebrow
	display: block
	margin-bottom: var(--sp-1)
	font-size: var(--fs-xs)
	color: var(--c-text-muted)

h2
	margin: 0
	font-size: var(--fs-lg)
	font-weight: 600
	line-height: 1.4

.favorite-action,
.cancel-action
	display: flex
	align-items: center
	justify-content: center
	width: 100%
	margin-top: var(--sp-3)
	padding: var(--sp-3) var(--sp-4)
	border-radius: var(--radius-sm)
	font-size: var(--fs-md)
	cursor: pointer

.favorite-action
	gap: var(--sp-2)
	color: var(--c-text)
	background: var(--c-bg)
	border: 1px solid var(--c-border)

	&:hover:not(:disabled),
	&.active
		border-color: var(--c-favorite)
		background: var(--c-favorite-bg)

	&:disabled
		opacity: 0.55
		cursor: wait

.star
	color: var(--c-favorite)
	font-size: var(--fs-xl)
	line-height: 1

.cancel-action
	color: var(--c-text-sub)
	background: none
	border: 1px solid transparent

	&:hover
		background: var(--c-overlay-1)

@media (max-width: 767px)
	dialog.channel-dialog
		width: calc(100vw - 24px)
		margin: auto var(--sp-3) var(--sp-3)
</style>
