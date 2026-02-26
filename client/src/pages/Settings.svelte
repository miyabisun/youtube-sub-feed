<script>
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import Spinner from '$lib/components/Spinner.svelte';
	import Toast from '$lib/components/Toast.svelte';

	let groups = $state([]);
	let channels = $state([]);
	let loading = $state(true);
	let toast = $state(null);
	let newGroupName = $state('');
	let editingGroup = $state(null);
	let selectedGroup = $state(null);
	let channelAssignments = $state({});

	// Drag state
	let dragIndex = $state(null);
	let dragOverIndex = $state(null);

	// Accordion state
	let expandedChannel = $state(null);
	let videoCache = $state({});

	// Filter state
	let showUnassignedOnly = $state(false);
	let filteredChannels = $derived(
		showUnassignedOnly ? channels.filter((ch) => !ch.group_names || channelAssignments[ch.id]) : channels
	);

	async function loadData() {
		try {
			[groups, channels] = await Promise.all([
				fetcher(`${config.path.api}/groups`),
				fetcher(`${config.path.api}/channels`),
			]);
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
		loading = false;
	}

	async function createGroup() {
		if (!newGroupName.trim()) return;
		try {
			const group = await fetcher(`${config.path.api}/groups`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ name: newGroupName.trim() }),
			});
			groups = [...groups, group];
			newGroupName = '';
			toast = { message: '作成しました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function renameGroup(id) {
		if (!editingGroup || !editingGroup.name.trim()) return;
		try {
			await fetcher(`${config.path.api}/groups/${id}`, {
				method: 'PATCH',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ name: editingGroup.name }),
			});
			groups = groups.map((g) => g.id === id ? { ...g, name: editingGroup.name } : g);
			editingGroup = null;
			toast = { message: '更新しました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function deleteGroup(id) {
		try {
			await fetcher(`${config.path.api}/groups/${id}`, { method: 'DELETE' });
			groups = groups.filter((g) => g.id !== id);
			if (selectedGroup === id) selectedGroup = null;
			toast = { message: '削除しました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function reorderGroups() {
		const order = groups.map((g) => g.id);
		try {
			await fetcher(`${config.path.api}/groups/reorder`, {
				method: 'PUT',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ order }),
			});
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function selectGroup(groupId) {
		selectedGroup = groupId;
		expandedChannel = null;
		try {
			const assignedIds = await fetcher(`${config.path.api}/groups/${groupId}/channels`);
			const assignedSet = new Set(assignedIds);
			channelAssignments = {};
			for (const ch of channels) {
				channelAssignments[ch.id] = assignedSet.has(ch.id);
			}
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function saveChannelAssignments() {
		if (!selectedGroup) return;
		const channelIds = Object.entries(channelAssignments).filter(([, v]) => v).map(([k]) => k);
		try {
			await fetcher(`${config.path.api}/groups/${selectedGroup}/channels`, {
				method: 'PUT',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ channelIds }),
			});
			toast = { message: '保存しました', type: 'success' };
			loadData();
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function toggleExpand(channelId) {
		if (expandedChannel === channelId) {
			expandedChannel = null;
			return;
		}
		expandedChannel = channelId;
		if (!videoCache[channelId]) {
			try {
				videoCache[channelId] = await fetcher(`${config.path.api}/channels/${channelId}/videos?limit=3`);
			} catch {
				videoCache[channelId] = [];
			}
		}
	}

	// Drag and drop handlers
	function onDragStart(index) {
		dragIndex = index;
	}

	function onDragOver(e, index) {
		e.preventDefault();
		dragOverIndex = index;
	}

	function onDrop(index) {
		if (dragIndex === null || dragIndex === index) return;
		const items = [...groups];
		const [moved] = items.splice(dragIndex, 1);
		items.splice(index, 0, moved);
		groups = items;
		dragIndex = null;
		dragOverIndex = null;
		reorderGroups();
	}

	function onDragEnd() {
		dragIndex = null;
		dragOverIndex = null;
	}

	loadData();
</script>

<div class="settings-page">
	{#if loading}
		<Spinner />
	{:else}
		<section class="section">
			<h2>グループ管理</h2>

			<div class="create-group">
				<input type="text" placeholder="新しいグループ名" bind:value={newGroupName} onkeydown={(e) => e.key === 'Enter' && createGroup()} />
				<button onclick={createGroup}>追加</button>
			</div>

			<div class="group-list">
				{#each groups as group, i (group.id)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="group-item"
						class:drag-over={dragOverIndex === i}
						draggable="true"
						ondragstart={() => onDragStart(i)}
						ondragover={(e) => onDragOver(e, i)}
						ondrop={() => onDrop(i)}
						ondragend={onDragEnd}
					>
						<span class="drag-handle">⠿</span>
						{#if editingGroup?.id === group.id}
							<input
								class="edit-input"
								type="text"
								bind:value={editingGroup.name}
								onkeydown={(e) => e.key === 'Enter' && renameGroup(group.id)}
								onblur={() => renameGroup(group.id)}
							/>
						{:else}
							<!-- svelte-ignore a11y_no_static_element_interactions -->
						<!-- svelte-ignore a11y_click_events_have_key_events -->
						<span class="group-name" onclick={() => editingGroup = { id: group.id, name: group.name }}>{group.name}</span>
						{/if}
						<div class="group-actions">
							<button class="btn-assign" class:active={selectedGroup === group.id} onclick={() => selectGroup(group.id)}>割当</button>
							<button class="btn-delete" onclick={() => deleteGroup(group.id)}>削除</button>
						</div>
					</div>
				{/each}
			</div>
		</section>

		{#if selectedGroup}
			<section class="section">
				<div class="assign-header">
					<h2>チャンネル割り当て: {groups.find(g => g.id === selectedGroup)?.name}</h2>
					<button class="btn-filter" class:active={showUnassignedOnly} onclick={() => showUnassignedOnly = !showUnassignedOnly}>未割当のみ</button>
				</div>
				<div class="channel-assign-list">
					{#each filteredChannels as ch (ch.id)}
						<div class="assign-card">
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<div class="assign-row" onclick={() => toggleExpand(ch.id)}>
								<input type="checkbox" bind:checked={channelAssignments[ch.id]} onclick={(e) => e.stopPropagation()} />
								{#if ch.thumbnail_url}
									<img class="channel-icon" src={ch.thumbnail_url} alt="" />
								{/if}
								<span class="channel-name">{ch.title}</span>
								{#if ch.group_names}
									<span class="group-labels">
										{#each ch.group_names.split(', ') as name}
											<span class="group-label">{name}</span>
										{/each}
									</span>
								{/if}
								<a class="yt-link" href="https://www.youtube.com/channel/{ch.id}" target="_blank" rel="noopener" onclick={(e) => e.stopPropagation()}>YT</a>
							</div>
							<div class="video-accordion" class:open={expandedChannel === ch.id}>
								<div class="video-accordion-inner">
									{#if expandedChannel === ch.id && videoCache[ch.id]}
										{#if videoCache[ch.id].length === 0}
											<p class="no-videos">動画がありません</p>
										{:else}
											<div class="video-thumbs">
												{#each videoCache[ch.id] as video}
													<a class="video-thumb" href="https://www.youtube.com/watch?v={video.id}" target="_blank" rel="noopener">
														{#if video.thumbnail_url}
															<img src={video.thumbnail_url} alt={video.title} loading="lazy" />
														{:else}
															<div class="thumb-placeholder"></div>
														{/if}
														<span class="video-title">{video.title}</span>
													</a>
												{/each}
											</div>
										{/if}
									{/if}
								</div>
							</div>
						</div>
					{/each}
				</div>
				<button class="save-btn" onclick={saveChannelAssignments}>保存</button>
			</section>
		{/if}
	{/if}
</div>

{#if toast}
	{#key Date.now()}
		<Toast message={toast.message} type={toast.type} />
	{/key}
{/if}

<style lang="sass">
.settings-page
	padding: var(--sp-3) var(--sp-4)

.section
	margin-bottom: var(--sp-6)

	h2
		font-size: var(--fs-lg)
		margin: 0 0 var(--sp-4)

.create-group
	display: flex
	gap: var(--sp-3)
	margin-bottom: var(--sp-4)

	input
		flex: 1
		padding: var(--sp-3) var(--sp-4)
		background: var(--c-surface)
		border: 1px solid var(--c-border)
		border-radius: var(--radius-md)
		color: var(--c-text)
		font-size: var(--fs-md)

		&:focus
			outline: none
			border-color: var(--c-accent)

	button
		padding: var(--sp-3) var(--sp-4)
		background: var(--c-accent)
		color: white
		border: none
		border-radius: var(--radius-md)
		cursor: pointer
		font-size: var(--fs-sm)

		&:hover
			background: var(--c-accent-hover)

.group-list
	display: flex
	flex-direction: column
	gap: var(--sp-2)

.group-item
	display: flex
	align-items: center
	gap: var(--sp-3)
	padding: var(--sp-3)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)

	&.drag-over
		border-color: var(--c-accent)

.drag-handle
	cursor: grab
	color: var(--c-text-muted)
	user-select: none
	font-size: var(--fs-lg)

.group-name
	flex: 1
	cursor: pointer

.edit-input
	flex: 1
	padding: var(--sp-2) var(--sp-3)
	background: var(--c-bg)
	border: 1px solid var(--c-accent)
	border-radius: var(--radius-sm)
	color: var(--c-text)
	font-size: var(--fs-md)

.group-actions
	display: flex
	gap: var(--sp-2)

	button
		padding: var(--sp-2) var(--sp-3)
		border: 1px solid var(--c-border)
		border-radius: var(--radius-sm)
		cursor: pointer
		font-size: var(--fs-xs)
		background: transparent

.btn-assign
	color: var(--c-text-sub)

	&:hover, &.active
		color: var(--c-accent)
		border-color: var(--c-accent-border)
		background: var(--c-accent-bg)

.btn-delete
	color: var(--c-danger-dim)

	&:hover
		color: var(--c-danger)
		border-color: var(--c-danger-border)
		background: var(--c-danger-bg)

.channel-assign-list
	margin-bottom: var(--sp-4)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)

.assign-header
	display: flex
	align-items: center
	gap: var(--sp-3)
	margin-bottom: var(--sp-4)

	h2
		margin: 0

.btn-filter
	padding: var(--sp-2) var(--sp-3)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	background: transparent
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	cursor: pointer
	white-space: nowrap

	&:hover
		color: var(--c-text)

	&.active
		color: var(--c-accent)
		border-color: var(--c-accent-border)
		background: var(--c-accent-bg)

.assign-card
	border-bottom: 1px solid var(--c-border)

	&:last-child
		border-bottom: none

.assign-row
	display: flex
	align-items: center
	gap: var(--sp-3)
	padding: var(--sp-3) var(--sp-4)
	cursor: pointer
	font-size: var(--fs-sm)

	&:hover
		background: var(--c-overlay-1)

	input
		accent-color: var(--c-accent)
		flex-shrink: 0

.channel-icon
	width: 32px
	height: 32px
	border-radius: 50%
	object-fit: cover
	flex-shrink: 0

.channel-name
	flex: 1
	min-width: 0
	overflow: hidden
	text-overflow: ellipsis
	white-space: nowrap

.group-labels
	display: flex
	gap: var(--sp-1)
	flex-shrink: 0

.group-label
	padding: 1px var(--sp-2)
	font-size: var(--fs-xs)
	color: var(--c-text-muted)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	white-space: nowrap

.yt-link
	flex-shrink: 0
	padding: var(--sp-1) var(--sp-3)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	text-decoration: none
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	white-space: nowrap

	&:hover
		color: var(--c-accent)
		border-color: var(--c-accent-border)

.video-accordion
	display: grid
	grid-template-rows: 0fr
	transition: grid-template-rows 0.25s ease

	&.open
		grid-template-rows: 1fr

.video-accordion-inner
	overflow: hidden

.video-thumbs
	display: flex
	gap: var(--sp-3)
	padding: var(--sp-3) var(--sp-4) var(--sp-4)

.video-thumb
	flex: 1
	min-width: 0
	text-decoration: none
	color: var(--c-text)

	img
		width: 100%
		aspect-ratio: 16 / 9
		object-fit: cover
		border-radius: var(--radius-sm)
		display: block

	&:hover img
		opacity: 0.8

.thumb-placeholder
	width: 100%
	aspect-ratio: 16 / 9
	background: var(--c-surface)
	border-radius: var(--radius-sm)

.video-title
	display: block
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	margin-top: var(--sp-1)
	overflow: hidden
	text-overflow: ellipsis
	white-space: nowrap

.no-videos
	padding: var(--sp-3) var(--sp-4)
	color: var(--c-text-muted)
	font-size: var(--fs-sm)
	margin: 0

.save-btn
	padding: var(--sp-3) var(--sp-5)
	background: var(--c-accent)
	color: white
	border: none
	border-radius: var(--radius-md)
	cursor: pointer
	font-size: var(--fs-md)

	&:hover
		background: var(--c-accent-hover)
</style>
