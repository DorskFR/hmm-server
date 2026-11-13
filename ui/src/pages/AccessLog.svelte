<script lang="ts">
	import {
		Button,
		Cluster,
		type Column,
		DataTable,
		EmptyState,
		Link,
		LoadMore,
		SectionHeader,
		Select,
		Stack,
		Text,
		Timestamp,
	} from '@dorsk/tsumikit';
	import { untrack } from 'svelte';
	import { type AccessEntry, api } from '../lib/api.ts';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import { Paged, Resource } from '../lib/query.svelte.ts';
	import { router } from '../lib/router.svelte.ts';

	const PAGE_SIZE = 50;

	const keys = new Resource(async () => (await api.keys()).keys);
	keys.load();

	const key = $derived(router.query.get('key'));
	const list = new Paged<AccessEntry>(async (cursor) => {
		const page = await api.access(key, cursor, PAGE_SIZE);
		return { items: page.entries, next: page.next_cursor };
	});
	$effect(() => {
		void key;
		untrack(() => list.reload());
	});

	const keyOptions = $derived([
		{ value: '', label: 'All keys' },
		...(keys.data ?? []).map((k) => ({
			value: k.id,
			label: k.name,
			hint: k.revoked_at ? 'revoked' : undefined,
		})),
	]);
	const keyName = $derived(keys.data?.find((k) => k.id === key)?.name);

	const columns = $derived<Column<AccessEntry>[]>([
		{ key: 'request', label: 'Request', role: 'title' },
		{ key: 'status', label: 'Status', width: '5rem', align: 'right', nowrap: true, role: 'meta' },
		...(key
			? []
			: [{ key: 'key', label: 'Key', width: '11rem', truncate: true, role: 'meta' } satisfies Column<AccessEntry>]),
		{ key: 'addr', label: 'Remote address', width: '10rem', nowrap: true, role: 'detail', hideBelow: 'md' },
		{ key: 'at', label: 'Time', width: '12rem', nowrap: true, align: 'right', role: 'meta' },
	]);

	const statusTone = (status: number) =>
		status >= 500 ? 'danger' : status >= 400 ? 'warn' : 'default';
</script>

<svelte:head><title>Access log · hmm admin</title></svelte:head>

{#snippet colRequest(e: AccessEntry)}
	<span class="request" title="{e.method} {e.path}">
		<Text size="sm" mono weight="medium">{e.method}</Text>
		<Text size="sm" mono truncate>{e.path}</Text>
	</span>
{/snippet}
{#snippet colStatus(e: AccessEntry)}
	<Text size="sm" numeric tone={statusTone(e.status)}>{e.status}</Text>
{/snippet}
{#snippet colKey(e: AccessEntry)}
	<Link href={router.href({ key: e.key_id })} tone="inherit">{e.key_name}</Link>
{/snippet}
{#snippet colAddr(e: AccessEntry)}
	{#if e.remote_addr}<Text size="sm" mono tone="muted">{e.remote_addr}</Text>{/if}
{/snippet}
{#snippet colAt(e: AccessEntry)}
	<Timestamp value={e.at} mode="datetime" size="sm" tone="muted" details={false} />
{/snippet}

<Stack gap="var(--sp-4)">
	<SectionHeader
		title="Access log"
		level={2}
		size="lg"
		subtitle="Every request made with an API key, newest first. Kept for 90 days."
	/>

	<Cluster gap="var(--sp-2)" align="center">
		<Select
			size="sm"
			width="auto"
			aria-label="Key"
			options={keyOptions}
			value={key ?? ''}
			onchange={(e) => router.setQuery({ key: e.currentTarget.value })}
		/>
	</Cluster>

	{#if list.error && list.items.length === 0}
		<ErrorState error={list.error} onretry={() => list.reload()} />
	{:else}
		<DataTable
			{columns}
			rows={list.items}
			rowKey={(e) => `${e.at}/${e.key_id}/${e.method}/${e.path}`}
			layout="fixed"
			responsive="stack"
			style="--dt-meta-font: var(--font-sans)"
			size="sm"
			loading={list.loading && !list.loaded}
			loadingRows={12}
			loadingLabel="Loading the access log"
			cellSnippets={{ request: colRequest, status: colStatus, key: colKey, addr: colAddr, at: colAt }}
		>
			{#snippet empty()}
				<EmptyState
					size="compact"
					icon="list"
					title={keyName ? `${keyName} has made no request` : 'No request has been made with a key'}
					description="Requests show up here as soon as an agent calls the admin API with its key. Browser sessions are not logged."
				>
					{#snippet action()}
						{#if key}<Button href="/access">Show all keys</Button>{/if}
					{/snippet}
				</EmptyState>
			{/snippet}
		</DataTable>
		{#if list.items.length > 0 && list.more !== 'done'}
			<LoadMore state={list.more} onload={() => list.loadMore()} label="Load older requests" />
		{/if}
	{/if}
</Stack>

<style>
	.request {
		display: flex;
		align-items: baseline;
		gap: var(--sp-2);
		min-width: 0;
	}
</style>
