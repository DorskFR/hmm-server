<script lang="ts">
	import {
		Badge,
		Button,
		Callout,
		Checkbox,
		Cluster,
		type Column,
		ConfirmModal,
		DataTable,
		EmptyState,
		Field,
		Input,
		Link,
		Modal,
		type RowAction,
		SectionHeader,
		Select,
		Stack,
		Text,
		Timestamp,
	} from '@dorsk/tsumikit';
	import { type ApiKey, api } from '../lib/api.ts';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import SecretModal from '../lib/components/SecretModal.svelte';
	import { errorText, SCOPES } from '../lib/labels.ts';
	import { Resource } from '../lib/query.svelte.ts';
	import { router } from '../lib/router.svelte.ts';
	import { projects } from '../lib/session.svelte.ts';

	const EXPIRY = [
		{ value: '', label: 'Never' },
		{ value: '30', label: 'In 30 days' },
		{ value: '90', label: 'In 90 days' },
		{ value: '365', label: 'In 1 year' },
	];

	const keys = new Resource(async () => (await api.keys()).keys);
	keys.load();

	let creating = $state(false);
	let submitting = $state(false);
	let formError = $state<unknown>(undefined);
	let name = $state('');
	let allProjects = $state(false);
	let picked = $state<Record<string, boolean>>({});
	let scopes = $state<Record<string, boolean>>({});
	let expiry = $state('');
	let created = $state<{ name: string; token: string } | null>(null);
	let revoking = $state<ApiKey | null>(null);

	const pickedProjects = $derived(Object.keys(picked).filter((p) => picked[p]));
	const pickedScopes = $derived(SCOPES.map((s) => s.id).filter((s) => scopes[s]));
	const valid = $derived(
		name.trim().length > 0 && (allProjects || pickedProjects.length > 0) && pickedScopes.length > 0,
	);

	function openCreate() {
		name = '';
		allProjects = false;
		picked = Object.fromEntries((projects.data ?? []).map((p) => [p.project, false]));
		scopes = Object.fromEntries(
			SCOPES.map((s) => [s.id, s.id === 'reports:read' || s.id === 'attachments:read']),
		);
		expiry = '';
		formError = undefined;
		creating = true;
	}

	async function create() {
		if (!valid || submitting) return;
		submitting = true;
		formError = undefined;
		try {
			const key = await api.createKey({
				name: name.trim(),
				projects: allProjects ? ['*'] : pickedProjects,
				scopes: pickedScopes,
				...(expiry
					? { expires_at: new Date(Date.now() + Number(expiry) * 86_400_000).toISOString() }
					: {}),
			});
			creating = false;
			created = { name: key.name, token: key.token };
			keys.load();
		} catch (e) {
			formError = e;
		} finally {
			submitting = false;
		}
	}

	async function revoke(key: ApiKey) {
		await api.revokeKey(key.id);
		revoking = null;
		await keys.load();
	}

	const keyState = (key: ApiKey) =>
		key.revoked_at
			? 'revoked'
			: key.expires_at && Date.parse(key.expires_at) <= Date.now()
				? 'expired'
				: 'active';

	const columns: Column<ApiKey>[] = [
		{ key: 'name', label: 'Key', role: 'title' },
		{ key: 'projects', label: 'Projects', width: '11rem', role: 'detail' },
		{ key: 'scopes', label: 'Scopes', width: '17rem', role: 'detail', hideBelow: 'md' },
		{ key: 'used', label: 'Last used', width: '8rem', nowrap: true, role: 'meta' },
		{ key: 'expires', label: 'Expires', width: '8rem', nowrap: true, role: 'detail', hideBelow: 'lg' },
	];

	const actions = (key: ApiKey): RowAction[] => [
		{
			id: 'access',
			label: `Access log of ${key.name}`,
			icon: 'list',
			onselect: () => router.navigate(`/access?key=${encodeURIComponent(key.id)}`),
		},
		{
			id: 'revoke',
			label: 'Revoke',
			icon: 'trash',
			danger: true,
			disabled: Boolean(key.revoked_at),
			onselect: () => (revoking = key),
		},
	];
</script>

<svelte:head><title>API keys · hmm admin</title></svelte:head>

{#snippet colName(key: ApiKey)}
	{@const s = keyState(key)}
	<span class="key">
		<span class="key-name">
			<Text weight="medium" tone={s === 'active' ? 'default' : 'muted'} truncate>{key.name}</Text>
			{#if s === 'revoked'}<Badge size="xs" tone="danger">Revoked</Badge>{/if}
			{#if s === 'expired'}<Badge size="xs" tone="warn">Expired</Badge>{/if}
		</span>
		<Text size="xs" tone="muted" mono>hmm_{key.prefix}_…</Text>
	</span>
{/snippet}
{#snippet colProjects(key: ApiKey)}
	{#if key.projects.includes('*')}
		<Text size="sm">All projects</Text>
	{:else}
		<Text size="sm">{key.projects.join(', ')}</Text>
	{/if}
{/snippet}
{#snippet colScopes(key: ApiKey)}
	<Cluster gap="var(--sp-1)">
		{#each key.scopes as scope (scope)}
			<Badge size="xs" mono tone={scope === 'admin' ? 'warn' : 'neutral'}>{scope}</Badge>
		{/each}
	</Cluster>
{/snippet}
{#snippet colUsed(key: ApiKey)}
	{#if key.last_used_at}
		<Link href="/access?key={encodeURIComponent(key.id)}" tone="inherit">
			<Timestamp value={key.last_used_at} mode="relative" short size="sm" details={false} />
		</Link>
	{:else}
		<Text size="sm" tone="muted">Never</Text>
	{/if}
{/snippet}
{#snippet colExpires(key: ApiKey)}
	{#if key.revoked_at}
		<Text size="sm" tone="muted">
			Revoked <Timestamp value={key.revoked_at} mode="short-iso" size="sm" tone="muted" />
		</Text>
	{:else if key.expires_at}
		<Timestamp value={key.expires_at} mode="short-iso" size="sm" />
	{:else}
		<Text size="sm" tone="muted">Never</Text>
	{/if}
{/snippet}

<Stack gap="var(--sp-4)">
	<SectionHeader
		title="API keys"
		level={2}
		size="lg"
		subtitle="Bearer tokens agents use to read and triage reports."
	>
		{#snippet actions()}
			<Button variant="primary" size="sm" onclick={openCreate}>New key</Button>
		{/snippet}
	</SectionHeader>

	{#if keys.error && !keys.data}
		<ErrorState error={keys.error} onretry={() => keys.load()} />
	{:else}
		<DataTable
			{columns}
			rows={keys.data ?? []}
			rowKey={(k) => k.id}
			layout="fixed"
			responsive="stack"
			stackActions="end"
			style="--dt-meta-font: var(--font-sans)"
			loading={!keys.data}
			loadingRows={4}
			loadingLabel="Loading keys"
			{actions}
			cellSnippets={{
				name: colName,
				projects: colProjects,
				scopes: colScopes,
				used: colUsed,
				expires: colExpires,
			}}
		>
			{#snippet empty()}
				<EmptyState
					size="compact"
					icon="key-round"
					title="No API key yet"
					description="A key lets an agent read and triage reports over the admin API. Create one and hand its token to the agent."
				>
					{#snippet action()}<Button onclick={openCreate}>New key</Button>{/snippet}
				</EmptyState>
			{/snippet}
		</DataTable>
	{/if}
</Stack>

{#if creating}
	<Modal title="New API key" size="md" busy={submitting} onclose={() => (creating = false)}>
		{#snippet body()}
			<Stack gap="var(--sp-4)">
				{#if formError}
					<Callout tone="danger" title="The key was not created">{errorText(formError)}</Callout>
				{/if}
				<Field label="Name" hint="Shown as the actor in events and in the access log.">
					<Input bind:value={name} placeholder="cctui-agent" autocomplete="off" onenter={create} />
				</Field>
				<Field label="Projects">
					<Stack gap="var(--sp-2)">
						<Checkbox label="All projects, including future ones" bind:checked={allProjects} />
						{#if !allProjects}
							{#each projects.data ?? [] as p (p.project)}
								<Checkbox label={p.project} bind:checked={picked[p.project]} />
							{/each}
						{/if}
					</Stack>
				</Field>
				<Field label="Scopes">
					<Stack gap="var(--sp-3)">
						{#each SCOPES as scope (scope.id)}
							<div>
								<Checkbox label={scope.label} bind:checked={scopes[scope.id]} />
								<Text as="p" size="xs" tone="muted" class="help">{scope.help}</Text>
							</div>
						{/each}
					</Stack>
				</Field>
				<Field label="Expires">
					<Select bind:value={expiry} options={EXPIRY} />
				</Field>
			</Stack>
		{/snippet}
		{#snippet footer()}
			<Button onclick={() => (creating = false)}>Cancel</Button>
			<Button variant="primary" disabled={!valid} loading={submitting} onclick={create}>Create key</Button>
		{/snippet}
	</Modal>
{/if}

{#if created}
	<SecretModal
		title="Key {created.name} created"
		label="Token"
		value={created.token}
		warning="Copy the token now. It is stored only as a hash and cannot be shown again."
		onclose={() => (created = null)}
	/>
{/if}

{#if revoking}
	{@const key = revoking}
	<ConfirmModal
		open
		tone="danger"
		title="Revoke {key.name}?"
		message="Requests using this key are refused at once. A revoked key cannot be restored; create a new one instead."
		confirmLabel="Revoke key"
		onconfirm={() => revoke(key)}
		oncancel={() => (revoking = null)}
	/>
{/if}

<style>
	.key {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.key-name {
		display: flex;
		align-items: center;
		gap: var(--sp-2);
		min-width: 0;
	}
	div :global(.help) {
		margin: 0;
		padding-inline-start: var(--sp-6);
	}
</style>
