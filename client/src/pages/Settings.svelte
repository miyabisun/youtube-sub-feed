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

	// Touch drag
	let touchStartY = null;
	let touchDragIndex = null;

	function onTouchStart(e, index) {
		touchStartY = e.touches[0].clientY;
		touchDragIndex = index;
	}

	function onTouchMove(e) {
		if (touchDragIndex === null) return;
		e.preventDefault();
	}

	function onTouchEnd(e, index) {
		touchDragIndex = null;
		touchStartY = null;
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
						<span class="group-name" ondblclick={() => editingGroup = { id: group.id, name: group.name }}>{group.name}</span>
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
				<h2>チャンネル割り当て: {groups.find(g => g.id === selectedGroup)?.name}</h2>
				<div class="channel-assign-list">
					{#each channels as ch (ch.id)}
						<label class="assign-item">
							<input type="checkbox" bind:checked={channelAssignments[ch.id]} />
							{#if ch.thumbnail_url}
								<img class="channel-icon" src={ch.thumbnail_url} alt="" />
							{/if}
							<span>{ch.title}</span>
						</label>
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
	max-width: 640px
	margin: 0 auto

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
	max-height: 400px
	overflow-y: auto
	margin-bottom: var(--sp-4)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)

.assign-item
	display: flex
	align-items: center
	gap: var(--sp-3)
	padding: var(--sp-3) var(--sp-4)
	cursor: pointer
	border-bottom: 1px solid var(--c-border)
	font-size: var(--fs-sm)

	&:last-child
		border-bottom: none

	&:hover
		background: var(--c-overlay-1)

	input
		accent-color: var(--c-accent)

.channel-icon
	width: 24px
	height: 24px
	border-radius: 50%
	object-fit: cover
	flex-shrink: 0

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
